"""Installer-only runtime preparation, using the downloaded embedded Python."""

import argparse
import ctypes
import hashlib
import json
import os
import re
from pathlib import Path
import queue
import runpy
import subprocess
import sys
import threading
import time
import traceback
from zipfile import ZipFile

NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)


def kill_tree(child):
    """Stop a tool and every helper it started; helpers may hold its pipes."""
    if os.name == "nt":
        try:
            subprocess.run(
                ["taskkill", "/PID", str(child.pid), "/T", "/F"],
                capture_output=True,
                creationflags=NO_WINDOW,
                timeout=30,
            )
        except (OSError, subprocess.SubprocessError):
            pass
    try:
        child.kill()
    except OSError:
        pass


def run_tool(argv, *, env=None, timeout, on_line=None, heartbeat=None, interval=5):
    """Stream a tool's output with a deadline and a heartbeat while it is quiet.

    Returns (exit code, last lines). Never blocks forever: a helper process that
    keeps the output pipe open after the tool exits is ignored after 10 seconds.
    """
    started = last_beat = time.monotonic()
    lines = queue.Queue()
    tail = []
    child = subprocess.Popen(
        [str(value) for value in argv],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        stdin=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
        env=env,
        creationflags=NO_WINDOW,
    )

    def pump():
        try:
            for line in child.stdout:
                lines.put(line.rstrip())
        finally:
            lines.put(None)

    threading.Thread(target=pump, daemon=True).start()
    finished = False
    exited_at = None
    while True:
        try:
            line = lines.get(timeout=1)
        except queue.Empty:
            line = ""
        if line is None:
            finished = True
            break
        if line:
            tail = (tail + [line])[-12:]
            if on_line:
                on_line(line)
        now = time.monotonic()
        if child.poll() is not None:
            exited_at = exited_at or now
            if now - exited_at > 10:
                break
        if now - started > timeout:
            kill_tree(child)
            # The killed tree releases the pipe; close it once the reader ends.
            ended = time.monotonic() + 5
            while time.monotonic() < ended:
                try:
                    if lines.get(timeout=0.5) is None:
                        child.stdout.close()
                        break
                except queue.Empty:
                    pass
            raise RuntimeError(
                f"{Path(str(argv[0])).name} did not finish within "
                f"{timeout // 60 or 1} minute(s) and was stopped. "
                + " ".join(tail[-3:])
            )
        if heartbeat and now - last_beat >= interval:
            heartbeat(int(now - started))
            last_beat = now
    try:
        code = child.wait(timeout=30)
    except subprocess.TimeoutExpired:
        kill_tree(child)
        code = child.wait(timeout=30)
    # Closing a pipe that a leftover helper still holds would wait for that
    # helper on Windows; the reader thread is a daemon and setup exits soon.
    if finished:
        child.stdout.close()
    return code, tail


def progress_reporter(start, end, label, emit):
    started = time.monotonic()
    last = None

    def report(completed, total, detail):
        nonlocal last
        now = time.monotonic()
        if last is None or completed == total or now - last >= 1:
            percent = start + (end - start) * completed // max(total, 1)
            emit(
                percent,
                f"{label}: {completed:,}/{total:,} files ({int(now - started)}s) - {detail}",
            )
            last = now

    return report


def extract_zip(archive, destination, progress=None):
    destination.mkdir(parents=True, exist_ok=True)
    root = destination.resolve()
    with ZipFile(archive) as source:
        entries = source.infolist()
        if progress:
            progress(0, len(entries), "Checking archive paths")
        for entry in entries:
            if not (root / entry.filename).resolve().is_relative_to(root):
                raise RuntimeError("Unsafe archive path")
        for index, entry in enumerate(entries, 1):
            try:
                source.extract(entry, root)
            except Exception as error:
                raise RuntimeError(
                    f"Could not extract equation file {entry.filename}: {error}"
                ) from error
            if progress:
                progress(index, len(entries), entry.filename)


def inventory(directory, progress=None):
    records = []
    files = [
        file
        for file in sorted(directory.rglob("*"))
        if file.is_file() and "__pycache__" not in file.parts and file.suffix != ".pyc"
    ]
    if progress:
        progress(0, len(files), "Checking extracted files")
    for index, file in enumerate(files, 1):
        try:
            with file.open("rb") as source:
                actual = hashlib.file_digest(source, "sha256").hexdigest()
        except OSError as error:
            raise RuntimeError(
                f"Could not verify equation file {file.relative_to(directory)}: {error}"
            ) from error
        records.append(
            {
                "path": file.relative_to(directory).as_posix(),
                "size": file.stat().st_size,
                "sha256": actual,
            }
        )
        if progress:
            progress(index, len(files), file.relative_to(directory).as_posix())
    encoded = json.dumps(records, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "algorithm": "sha256",
        "tree_sha256": hashlib.sha256(encoded).hexdigest(),
        "files": records,
    }


def verify_math_payload(directory, manifest, progress=None):
    expected = json.loads(manifest.read_text(encoding="utf-8"))
    actual = inventory(directory, progress)
    if actual != expected:
        expected_files = {item["path"]: item for item in expected["files"]}
        actual_files = {item["path"]: item for item in actual["files"]}
        changed = next(
            (
                name
                for name in sorted(expected_files.keys() | actual_files.keys())
                if expected_files.get(name) != actual_files.get(name)
            ),
            "inventory metadata",
        )
        raise RuntimeError(
            f"The bundled equation snapshot failed verification: {changed}"
        )
    return actual


def native_probe(argv, env, report=None, timeout=60):
    """Keep native hangs/crashes outside setup, and retain their diagnostics."""
    name = Path(argv[-1] if "--ghostscript" in argv else argv[0]).name
    started = time.monotonic()
    if report:
        report(f"Checking {name}")
    with subprocess.Popen(
        [str(value) for value in argv],
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdin=subprocess.DEVNULL,
        creationflags=NO_WINDOW,
    ) as child:
        try:
            while True:
                remaining = timeout - (time.monotonic() - started)
                if remaining <= 0:
                    raise subprocess.TimeoutExpired(argv, timeout)
                try:
                    stdout, stderr = child.communicate(timeout=min(2, remaining))
                    break
                except subprocess.TimeoutExpired:
                    if report:
                        report(f"Checking {name} ({int(time.monotonic() - started)}s)")
        except subprocess.TimeoutExpired as error:
            kill_tree(child)
            try:
                stdout, stderr = child.communicate(timeout=10)
            except subprocess.TimeoutExpired:
                stdout, stderr = "", "A helper process kept running."
            raise RuntimeError(
                f"Checking {name} timed out after {timeout}s. "
                f"{(stderr or stdout)[-1000:]}"
            ) from error
        if child.returncode:
            raise RuntimeError(
                f"Checking {name} failed (exit code {child.returncode}, "
                f"0x{child.returncode & 0xFFFFFFFF:08X}): {(stderr or stdout)[-1000:]}"
            )
        return stdout


def ghostscript_revision(binary):
    """Called only in a disposable native probe process."""
    if os.name == "nt":
        # Suppress native loader error dialogs that otherwise block unattended setup.
        ctypes.windll.kernel32.SetErrorMode(0x0001 | 0x0002 | 0x8000)

    class GhostscriptRevision(ctypes.Structure):
        _fields_ = [
            ("product", ctypes.c_char_p),
            ("copyright", ctypes.c_char_p),
            ("revision", ctypes.c_long),
            ("revisiondate", ctypes.c_long),
        ]

    revision = GhostscriptRevision()
    ghostscript = ctypes.CDLL(str(binary))
    query = ghostscript.gsapi_revision
    query.argtypes = [ctypes.POINTER(GhostscriptRevision), ctypes.c_int]
    query.restype = ctypes.c_int
    if query(ctypes.byref(revision), ctypes.sizeof(revision)):
        raise RuntimeError("Ghostscript version query failed")
    return revision.revision


def verify_math_engines(bin_dir, env, native_lock, report=None):
    """Reject a moving mirror whose native renderers exceed this release's sources."""
    versions = {}
    for name, pattern in (
        ("latex.exe", r"MiKTeX 26\.5(?: Portable)?\)"),
        ("dvisvgm.exe", r"(?m)^dvisvgm 3\.6\s*$"),
    ):
        output = native_probe([bin_dir / name, "--version"], env, report)
        if not re.search(pattern, output):
            raise RuntimeError(
                "The equation renderer changed upstream. Download a newer Manim Studio release; this release includes sources for MiKTeX 26.5, dvisvgm 3.6 and Ghostscript 9.25."
            )
        versions[name] = output.splitlines()[0]
    revision = native_probe(
        [
            sys.executable,
            "-u",
            Path(__file__).resolve(),
            "--ghostscript",
            bin_dir / "mgsdll64.dll",
        ],
        env,
        report,
    ).strip()
    if revision != "925":
        raise RuntimeError(
            "Ghostscript changed upstream. Download a newer Manim Studio release to obtain its matching native sources."
        )
    versions["mgsdll64.dll"] = revision
    if report:
        report("Checking native equation file hashes")
    expected = json.loads(native_lock.read_text(encoding="utf-8"))["files"]
    actual_names = {
        file.name
        for file in bin_dir.iterdir()
        if file.is_file() and file.suffix.lower() in {".exe", ".dll"}
    }
    if actual_names != set(expected):
        raise RuntimeError(
            "The native equation renderer file set changed upstream. Download a newer Manim Studio release to obtain its matching sources."
        )
    for file in bin_dir.iterdir():
        if file.is_file() and file.suffix.lower() in {".exe", ".dll"}:
            with file.open("rb") as source:
                actual = hashlib.file_digest(source, "sha256").hexdigest()
            if expected.get(file.name) != actual:
                raise RuntimeError(
                    "The equation renderer changed upstream ("
                    + file.name
                    + "). Download a newer Manim Studio release to obtain its matching native sources."
                )
    return versions


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--downloads", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()
    downloads, destination = args.downloads.resolve(), args.destination.resolve()
    if destination.parent != downloads or destination.name != "prepared":
        raise RuntimeError(
            "Runtime preparation must stay in the installer temporary folder"
        )
    args.log.parent.mkdir(parents=True, exist_ok=True)
    with args.log.open("w", encoding="utf-8") as log:

        def say(message):
            print(message, flush=True)
            log.write(message + "\n")
            log.flush()

        def stage(percent, message):
            say(f"STAGE|{percent}|{message}")

        def command(argv, env=None, *, timeout, on_line=None, heartbeat=None):
            say("Running: " + str(argv[0]))

            def record(line):
                log.write(line + "\n")
                log.flush()
                if any(
                    term in line.lower()
                    for term in (
                        "downloading ",
                        "installing ",
                        "successfully installed",
                    )
                ):
                    print(line[:220], flush=True)
                if on_line:
                    on_line(line)

            code, tail = run_tool(
                argv, env=env, timeout=timeout, on_line=record, heartbeat=heartbeat
            )
            if code:
                raise RuntimeError("The setup tool failed:\n" + "\n".join(tail))

        def pip_progress(requirements):
            """Map pip's output to 15-58% so long installs never look frozen."""
            total = max(
                1,
                sum(
                    1
                    for line in requirements.read_text(encoding="utf-8").splitlines()
                    if re.match(r"^[A-Za-z0-9][\w.-]*==", line) and "darwin" not in line
                ),
            )
            state = {"seen": 0, "installing": None}

            def on_line(line):
                if line.startswith(("Collecting ", "Processing ")):
                    state["seen"] = min(total, state["seen"] + 1)
                    stage(
                        15 + 35 * state["seen"] // total,
                        f"Downloading Manim libraries: {state['seen']} of {total}",
                    )
                elif line.startswith("Installing collected packages"):
                    state["installing"] = time.monotonic()
                    stage(50, "Installing Manim libraries. This can take a few minutes")

            def heartbeat(elapsed):
                if state["installing"] is None:
                    stage(
                        15 + 35 * state["seen"] // total,
                        f"Downloading Manim libraries: {state['seen']} of {total} ({elapsed}s)",
                    )
                else:
                    waited = int(time.monotonic() - state["installing"])
                    stage(
                        min(58, 50 + waited // 20),
                        f"Installing Manim libraries, still working ({waited}s). "
                        "Antivirus scans can make this step slow",
                    )

            return on_line, heartbeat

        try:
            stage(5, "Checking downloaded tools")
            for item in json.loads((downloads / "downloads.json").read_text()):
                with (downloads / item["name"]).open("rb") as source:
                    actual = hashlib.file_digest(source, "sha256").hexdigest()
                if actual != item["sha256"]:
                    raise RuntimeError("Download verification failed: " + item["name"])

            runtime = destination / "runtime"
            (runtime / "Lib" / "site-packages").mkdir(parents=True, exist_ok=True)
            (runtime / "python312._pth").write_text(
                "python312.zip\n.\nLib/site-packages\n../app\nimport site\n"
            )
            extract_zip(downloads / "pip.whl", downloads / "pip-package")
            stage(15, "Downloading and installing Manim libraries")
            pip_line, pip_heartbeat = pip_progress(
                downloads / "requirements-online.txt"
            )
            command(
                [
                    runtime / "python.exe",
                    "-u",
                    Path(__file__),
                    "--pip",
                    downloads / "pip-package",
                    "install",
                    "--target",
                    runtime / "Lib" / "site-packages",
                    "--require-hashes",
                    "--only-binary=:all:",
                    "--find-links",
                    downloads / "wheels",
                    "--no-compile",
                    "--no-cache-dir",
                    "--disable-pip-version-check",
                    "--retries",
                    "5",
                    "--timeout",
                    "60",
                    "-r",
                    downloads / "requirements-online.txt",
                ],
                timeout=60 * 60,
                on_line=pip_line,
                heartbeat=pip_heartbeat,
            )

            stage(60, "Preparing the verified equation payload")
            profile = json.loads(
                (downloads / "runtime-profile.json").read_text(encoding="utf-8")
            )
            if profile.get("webview2") != "evergreen" or profile.get("math") not in {
                "minimal",
                "full",
                "none",
            }:
                raise RuntimeError("Unsupported runtime profile")
            math_inventory = None
            versions = {}
            if profile["math"] != "none":
                math = destination / "math"
                extract_zip(
                    downloads / "math-runtime.zip",
                    math,
                    progress_reporter(60, 78, "Extracting equation tools", stage),
                )
                math_inventory = verify_math_payload(
                    math,
                    downloads / "math-inventory.json",
                    progress_reporter(78, 86, "Verifying equation tools", stage),
                )
                bin_dir = math / "texmfs" / "install" / "miktex" / "bin" / "x64"
                env = dict(
                    os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"]
                )
                versions = verify_math_engines(
                    bin_dir,
                    env,
                    downloads / "tex-native-lock.json",
                    lambda message: stage(88, message),
                )

            stage(92, "Recording installed tool versions and package snapshot")
            provenance = {
                "schema": 1,
                "downloads": json.loads((downloads / "downloads.json").read_text()),
                "requirements_sha256": hashlib.sha256(
                    (downloads / "requirements-online.txt").read_bytes()
                ).hexdigest(),
                "runtime_profile": profile,
                "math_inventory": math_inventory,
                "verified_math_engines": versions,
            }
            (destination / "INSTALLED-RUNTIME-PROVENANCE.json").write_text(
                json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
            )

            stage(95, "Checking the render engine")
            command(
                [
                    runtime / "python.exe",
                    "-c",
                    'import manim, webview, av, typst, kokoro_onnx; assert manim.__version__ == "0.21.0"; assert typst.compile(b"$x^2$", format="svg").startswith(b"<svg")',
                ],
                timeout=10 * 60,
                heartbeat=lambda elapsed: stage(
                    95, f"Checking the render engine, still working ({elapsed}s)"
                ),
            )
            # The verified private audio tools ship in the installer's core files.
            if profile["math"] != "none":
                for tool in ("latex.exe", "dvisvgm.exe"):
                    stage(97, f"Checking {tool}")
                    command([bin_dir / tool, "--version"], env, timeout=3 * 60)
            stage(100, "All tools are ready. Installing Manim Studio...")
        except Exception as error:
            say("ERROR|" + str(error).replace("\n", " ")[:800])
            say(traceback.format_exc())
            raise


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--ghostscript":
        print(ghostscript_revision(Path(sys.argv[2])), flush=True)
    elif len(sys.argv) > 1 and sys.argv[1] == "--pip":
        sys.path.insert(0, sys.argv[2])
        sys.argv = ["pip"] + sys.argv[3:]
        runpy.run_module("pip", run_name="__main__")
    else:
        main()
