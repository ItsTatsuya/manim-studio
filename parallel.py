"""Render long scenes with several Manim processes and join their animations.

Manim renders one animation after another in a single thread, so most of a
laptop sits idle during a long render. Here every helper process runs the whole
script but only renders its share of the animations (blocks of `BLOCK` plays,
dealt out in turn) and skips the rest, which for Manim means jumping straight to
the animation's end state. The partial movie files are then joined exactly the
way Manim joins them.

Skipping must leave the scene in the state real playback would have produced. Each
helper therefore writes a fingerprint of the scene at the start of every block;
a block was rendered by one helper and skipped by all the others, so equal
fingerprints prove that skipping and rendering agree there. Any disagreement, helper
failure or unsupported feature makes `render` return False and the caller renders
normally, so this is an accelerator and never changes what the video contains.
"""

from __future__ import annotations

import ast
import ctypes
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

from processes import popen_options, stop_process

BLOCK = 2
# A static wait renders one frame and repeats it, so it is far cheaper than motion.
FROZEN_WEIGHT = 0.08
# Fixed cost of starting an animation, expressed in rendered frames.
ANIMATION_COST = 4.0
# Below this much estimated work the helpers' start-up costs more than they save.
MIN_WORK = 1500.0
# Memory one helper needs, and what stays free for Studio, Windows and narration.
WORKER_MEGABYTES = 800
RESERVED_MEGABYTES = 1500
# Beyond a few helpers the processes fight over memory bandwidth and cache:
# measured scaling stops at about 4 on an 8-core desktop.
MAX_WORKERS = 4
BELOW_NORMAL_PRIORITY = 0x00004000
# Features whose output is assembled outside the animations' partial movies.
UNSUPPORTED = {"add_sound", "next_section", "add_subcaption", "interactive_embed"}
UNSUPPORTED_FLAGS = {"-t", "--transparent", "-s", "--save_last_frame", "-n", "--gif"}


def available_megabytes() -> int | None:
    """Free physical memory, or None when the platform cannot say."""
    if os.name != "nt":
        try:
            return os.sysconf("SC_AVPHYS_PAGES") * os.sysconf("SC_PAGE_SIZE") >> 20
        except (AttributeError, OSError, ValueError):
            return None

    class Status(ctypes.Structure):
        _fields_ = [
            ("length", ctypes.c_uint32),
            ("load", ctypes.c_uint32),
            ("total", ctypes.c_uint64),
            ("available", ctypes.c_uint64),
            ("total_page_file", ctypes.c_uint64),
            ("available_page_file", ctypes.c_uint64),
            ("total_virtual", ctypes.c_uint64),
            ("available_virtual", ctypes.c_uint64),
            ("available_extended", ctypes.c_uint64),
        ]

    status = Status(length=ctypes.sizeof(Status))
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        return None
    return int(status.available >> 20)


def worker_count(setting: str) -> int:
    """How many helper processes to use: 'auto' follows cores and free memory."""
    if setting not in {"", "auto"}:
        try:
            return max(1, min(int(setting), MAX_WORKERS))
        except ValueError:
            return 1
    # os.cpu_count() counts hyper-threads; a helper wants a physical core.
    count = min(MAX_WORKERS, max(1, (os.cpu_count() or 1) // 2))
    free = available_megabytes()
    if free is not None:
        count = min(count, max(1, (free - RESERVED_MEGABYTES) // WORKER_MEGABYTES))
    return count


def inspect_script(script: Path) -> tuple[bool, bool]:
    """(worth splitting, uses LaTeX) from the source alone, without running it.

    Scripts using features that cannot be joined from partial movies are
    reported as not worth splitting.
    """
    try:
        tree = ast.parse(script.read_text(encoding="utf-8"))
    except (OSError, SyntaxError, ValueError, RecursionError):
        return False, False
    calls, looped, latex = 0, False, False
    for node in ast.walk(tree):
        if isinstance(node, (ast.For, ast.While, ast.ListComp, ast.GeneratorExp)):
            looped = True
        elif isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr in UNSUPPORTED:
                return False, False
            if node.func.attr in {"play", "wait"}:
                calls += 1
            if any(keyword.arg == "subcaption" for keyword in node.keywords):
                return False, False
        if isinstance(node, ast.Name) and node.id in {"Tex", "MathTex"}:
            latex = True
        elif isinstance(node, ast.Attribute) and node.attr in {"Tex", "MathTex"}:
            latex = True
    return looped or calls >= 12, latex


def with_options(argv, **options):
    """Copy Manim's arguments, replacing the value of the given flags."""
    result = list(argv)
    for flag, value in options.items():
        flag = "--" + flag
        if flag in result:
            result[result.index(flag) + 1] = str(value)
    return result


def fingerprint(scene, media: str) -> str:
    """A digest of everything that decides what the scene looks like next.

    Process-specific values (object ids, the media folder) are left out, as is
    the camera's pixel buffer, which holds whatever frame was drawn last.
    """
    import hashlib

    import numpy as np
    from manim.mobject.mobject import Mobject

    digest = hashlib.blake2b(digest_size=16)
    update = digest.update
    seen: set[int] = set()
    prefixes = {media, media.replace("\\", "/"), media.replace("/", "\\")}
    buffers = {
        "pixel_array",
        "background",
        "original_background",
        "pixel_array_to_cairo_context",
    }

    def put(value, depth=0, path=""):
        if isinstance(value, np.ndarray):
            if value.dtype.hasobject:
                update(repr(value.tolist()).encode())
            else:
                update(f"{value.dtype}{value.shape}".encode())
                update(np.ascontiguousarray(value).data)
        elif value is None or isinstance(value, (bool, int, float, bytes, np.generic)):
            update(repr(value).encode())
        elif isinstance(value, os.PathLike):
            put(os.fspath(value), depth, path)
        elif isinstance(value, str):
            for prefix in prefixes:
                if prefix in value:
                    value = value.replace(prefix, "<media>")
            update(value.encode("utf-8", "replace"))
        elif isinstance(value, (list, tuple)):
            update(b"[")
            for position, item in enumerate(value):
                put(item, depth + 1, f"{path}[{position}]")
        elif isinstance(value, (set, frozenset)):
            update(b"(")
            for item in sorted(value, key=repr):
                put(item, depth + 1, path + "{}")
        elif isinstance(value, dict):
            update(b"{")
            for key in sorted(value, key=repr):
                if key != "original_id":
                    put(key, depth + 1, f"{path}.{key}?")
                    put(value[key], depth + 1, f"{path}.{key}")
        elif isinstance(value, Mobject):
            if id(value) in seen:
                update(b"@")
                return
            seen.add(id(value))
            update(type(value).__name__.encode())
            put(vars(value), depth + 1, f"{path}<{type(value).__name__}>")
        elif callable(value):
            update(getattr(value, "__qualname__", type(value).__name__).encode())
        elif depth < 5 and hasattr(value, "__dict__"):
            update(type(value).__name__.encode())
            put(vars(value), depth + 1, f"{path}<{type(value).__name__}>")
        else:
            update(type(value).__name__.encode())

    put(scene.mobjects)
    put(getattr(scene, "foreground_mobjects", None))
    put([getattr(updater, "__qualname__", "") for updater in scene.updaters])
    camera = {
        key: value
        for key, value in vars(scene.camera).items()
        if key not in buffers
        and not (isinstance(value, np.ndarray) and value.ndim >= 3)
    }
    put(camera)
    return digest.hexdigest()


def _launch(argv, env, log):
    options = popen_options()
    if os.name == "nt":
        # Keep the rest of the laptop responsive while every core is busy.
        options["creationflags"] |= BELOW_NORMAL_PRIORITY
    return subprocess.Popen(
        [sys.executable, "-u", *argv],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=subprocess.STDOUT,
        **options,
    )


def _stop(processes):
    for process in processes:
        # A virtual environment's python.exe starts the real interpreter as a child.
        if process.poll() is None:
            try:
                stop_process(process)
            except (OSError, subprocess.SubprocessError):
                process.kill()
    for process in processes:
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass


def _complete(path: Path) -> bool:
    """A partial movie of a killed helper has no index; refuse it."""
    import av

    try:
        with av.open(str(path)) as movie:
            return bool(movie.streams.video) and movie.duration is not None
    except (av.error.FFmpegError, OSError, ValueError):
        return False


def join(files, output: Path):
    """Concatenate partial movies without re-encoding, as Manim does."""
    import av

    listing = output.with_name(output.stem + "-files.txt")
    listing.write_text(
        "# This file is used internally by FFMPEG.\n"
        + "".join(f"file 'file:{Path(path).as_posix()}'\n" for path in files),
        encoding="utf-8",
    )
    try:
        source = av.open(
            str(listing), options={"safe": "0", "an": "1"}, format="concat"
        )
        try:
            stream = source.streams.video[0]
            target = av.open(str(output), mode="w")
            try:
                import manim

                target.metadata["comment"] = (
                    f"Rendered with Manim Community v{manim.__version__}"
                )
                template = target.add_stream_from_template(template=stream)
                for packet in source.demux(stream):
                    if packet.dts is None:  # Skip the demuxer's flushing packets.
                        continue
                    packet.dts = None  # Let libav order packets across files.
                    packet.stream = template
                    target.mux(packet)
            finally:
                target.close()
        finally:
            source.close()
    finally:
        listing.unlink(missing_ok=True)


def _read(path: Path) -> dict[str, str]:
    """Fingerprints a helper has logged so far (complete lines only)."""
    try:
        lines = path.read_text(encoding="utf-8").split("\n")[:-1]
    except OSError:
        return {}
    return dict(line.split(" ", 1) for line in lines if " " in line)


def render(argv, *, env, stage, abort, setting="auto", directory: Path) -> bool:
    """Render the animation with helper processes and write the movie file.

    Returns True when the finished movie is in place. Returns False when the
    scene is too small, unsupported or any check failed; nothing has been
    changed in that case and the caller should render normally.
    """
    flags = set(argv)
    if (
        "--format" not in flags
        or argv[argv.index("--format") + 1] != "mp4"
        or flags & UNSUPPORTED_FLAGS
        or "--renderer" in flags
        and argv[argv.index("--renderer") + 1] != "cairo"
    ):
        return False
    workers = worker_count(setting)
    long, latex = inspect_script(Path(argv[-2]))
    if workers < 2 or not long:
        return False
    media = Path(argv[argv.index("--media_dir") + 1])
    scene = argv[-1]
    directory.mkdir(parents=True, exist_ok=True)
    processes, logs = [], []

    def start(name, role, index, *, seed=None):
        folder = directory / name
        folder.mkdir(parents=True, exist_ok=True)
        if seed is not None:
            for sub in ("Tex", "texts"):
                if (seed / sub).is_dir():
                    shutil.copytree(seed / sub, folder / sub, dirs_exist_ok=True)
        output = directory / f"{name}.out"
        spec = {
            "role": role,
            "output": str(output),
            "index": index,
            "count": workers,
            "block": BLOCK,
        }
        log = (directory / f"{name}.log").open("wb")
        logs.append(log)
        options = dict(media_dir=folder, progress_bar="none")
        child = ["--disable_caching", "--max-inflight-encoders", "2"]
        arguments = with_options(argv[1:], **options)
        arguments[0:0] = child
        process = _launch(
            [argv[0], *arguments], dict(env, MANIM_STUDIO_HELPER=json.dumps(spec)), log
        )
        processes.append(process)
        return folder, output, process

    try:
        scout = start("scout", "plan", 0)
        if latex:
            # LaTeX compiles once, here, and the helpers reuse its results.
            stage("Preparing the scene…")
            while scout[2].poll() is None:
                abort()
                time.sleep(0.1)
        plan = None
        if latex and scout[2].returncode == 0 and scout[1].is_file():
            plan = json.loads(scout[1].read_text(encoding="utf-8"))
            if _work(plan)[0] < MIN_WORK:
                return False
        elif latex:
            return False
        shards = [
            start(f"shard{index}", "shard", index, seed=scout[0] if latex else None)
            for index in range(workers)
        ]
        prints = [directory / f"shard{index}.out" for index in range(workers)]
        total, costs, shown = None, None, -1
        agreed: dict[str, str] = {}
        while any(process.poll() is None for _, _, process in shards):
            abort()
            if any(process.poll() not in (None, 0) for _, _, process in shards):
                return False
            if plan is None and scout[2].poll() == 0 and scout[1].is_file():
                plan = json.loads(scout[1].read_text(encoding="utf-8"))
                if _work(plan)[0] < MIN_WORK:
                    return False
            if plan is not None and costs is None:
                total, costs = _work(plan)
            for path in prints:
                for key, value in _read(path).items():
                    if agreed.setdefault(key, value) != value:
                        return False
            if costs is not None:
                done = 0.0
                for folder, _, _ in shards:
                    for movie in folder.glob("videos/*/*/partial_movie_files/*/*.mp4"):
                        number = int(movie.stem.rsplit("_", 1)[-1])
                        if number < len(costs):
                            done += costs[number]
                percent = min(99, int(100 * done / total))
            else:
                percent = 0
            if percent != shown:
                shown = percent
                stage(f"Rendering animations in parallel · {percent}%")
            time.sleep(0.25)

        if any(process.returncode for _, _, process in shards):
            return False
        # Every helper must have reached the end of the scene and agree on it.
        results = [_read(path) for path in prints]
        ends = {key for result in results for key in result if key.startswith("end")}
        if len(ends) != 1 or any(next(iter(ends)) not in result for result in results):
            return False
        if any(result != results[0] for result in results[1:]):
            return False
        plays = int(next(iter(ends)).split("_")[1])
        movies, destination = {}, None
        for folder, _, _ in shards:
            for movie in folder.glob("videos/*/*/partial_movie_files/*/*.mp4"):
                movies[int(movie.stem.rsplit("_", 1)[-1])] = movie
                destination = movie.relative_to(folder).parents[2]
        if plays == 0 or sorted(movies) != list(range(plays)):
            return False
        if not all(_complete(movie) for movie in movies.values()):
            return False
        stage("Putting your video together…")
        name = argv[argv.index("-o") + 1] if "-o" in argv else scene
        output = media / destination / f"{name}.mp4"
        output.parent.mkdir(parents=True, exist_ok=True)
        join([movies[number] for number in range(plays)], output)
        return True
    finally:
        _stop(processes)
        for log in logs:
            log.close()
        shutil.rmtree(directory, ignore_errors=True)


def _work(plan: dict) -> tuple[float, list[float]]:
    fps = float(plan["fps"])
    costs = [
        ANIMATION_COST
        + max(1, round(duration * fps)) * (FROZEN_WEIGHT if frozen else 1.0)
        for duration, frozen in plan["animations"]
    ]
    return sum(costs), costs
