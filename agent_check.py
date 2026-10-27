"""Check a Studio project the way a render would, without writing video.

Agents run this from a project folder after editing. It reports syntax and
runtime errors, missing assets, and whether the narration fits the animation,
and saves a contact sheet of the scene so agents need not render to see it.
Usage: python agent_check.py [script.py] [SceneName]
"""

from __future__ import annotations

import ast
import importlib.util
import os
from pathlib import Path
import shutil
import sys
import tempfile
import traceback

sys.dont_write_bytecode = True  # Keep project folders free of __pycache__.
APP_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(APP_ROOT))

import narration  # noqa: E402
from runtime import DATA_ROOT  # noqa: E402
import scenes  # noqa: E402
import styles  # noqa: E402

# Render-like font lookup before Manim loads Pango (inherited from Studio).
styles.configure_fonts(DATA_ROOT)

WORDS_PER_SECOND = 2.5
# One image the agent can look at instead of rendering and grabbing frames.
# Hidden, so render cache keys, exports and asset lists never include it.
PREVIEW = Path(".check") / "preview.png"
PREVIEW_FRAMES = 15
PREVIEW_COLUMNS = 3
THUMBNAIL = (400, 225)


def referenced_assets(tree: ast.AST) -> list[str]:
    return sorted(
        {
            node.value.replace("\\", "/")
            for node in ast.walk(tree)
            if isinstance(node, ast.Constant)
            and isinstance(node.value, str)
            and node.value.replace("\\", "/").startswith("assets/")
        }
    )


def narration_end(text: str) -> float:
    """Estimate when the last narration line finishes speaking."""
    cursor = 0.0
    for index, segment in enumerate(narration.parse_timeline(text)):
        start = segment["start"]
        if start is None or start < cursor:
            start = cursor + (narration.GAP if index else 0.0)
        cursor = start + len(segment["text"].split()) / WORDS_PER_SECOND
    return cursor


def _label(mobject) -> str:
    text = (
        getattr(mobject, "original_text", None)
        or getattr(mobject, "tex_string", None)
        or getattr(mobject, "text", None)
    )
    text = " ".join(str(text or type(mobject).__name__).split())
    return text if len(text) <= 32 else text[:31] + "…"


def _box(mobject):
    from manim import DL, UR

    (x0, y0, _), (x1, y1, _) = (
        mobject.get_critical_point(DL),
        mobject.get_critical_point(UR),
    )
    return x0, y0, x1, y1


def _shared(a, b) -> float:
    """How much of the smaller of two boxes the other one covers (0 to 1)."""
    width = min(a[2], b[2]) - max(a[0], b[0])
    height = min(a[3], b[3]) - max(a[1], b[1])
    if width <= 0 or height <= 0:
        return 0.0
    smaller = min((a[2] - a[0]) * (a[3] - a[1]), (b[2] - b[0]) * (b[3] - b[1]))
    return width * height / smaller if smaller > 1e-6 else 0.0


def _visible(mobject) -> bool:
    try:
        return (
            mobject.has_points()
            and max(mobject.get_fill_opacity(), mobject.get_stroke_opacity()) > 0.05
        )
    except Exception:
        return mobject.has_points()


def _texts(mobjects) -> list:
    """Visible text and equations, without descending into their letters."""
    from manim import MarkupText, SingleStringMathTex, Text

    kinds = [Text, MarkupText, SingleStringMathTex]
    try:
        from manim import MathTypst, Typst

        kinds += [MathTypst, Typst]
    except ImportError:
        pass
    found = []

    def walk(mobject):
        if isinstance(mobject, tuple(kinds)):
            if any(_visible(part) for part in mobject.family_members_with_points()):
                found.append(mobject)
            return
        for part in mobject.submobjects:
            walk(part)

    for mobject in mobjects:
        walk(mobject)
    return found


def watch_layout(script: Path, problems: list[str]):
    """Report text that overlaps, leaves the frame or ghosts through a crossfade.

    Checked after every play() and wait() of the dry run, like a viewer would
    see the settled frame.
    """
    from manim import FadeOut, Scene, config

    original_play = Scene.play
    seen = set()

    def where() -> str:
        for frame in reversed(traceback.extract_stack()):
            if Path(frame.filename).resolve() == script.resolve():
                return f"line {frame.lineno}"
        return "the scene"

    def report(key, message):
        if key not in seen and len(seen) < 6:
            seen.add(key)
            problems.append(message)

    def play(self, *args, **kwargs):
        animations = [arg for arg in args if hasattr(arg, "mobject")]
        leaving = [anim.mobject for anim in animations if isinstance(anim, FadeOut)]
        on_screen = {id(part) for item in self.mobjects for part in item.get_family()}
        arriving = [
            anim.mobject
            for anim in animations
            if not isinstance(anim, FadeOut) and id(anim.mobject) not in on_screen
        ]
        for old in _texts(leaving):
            for new in _texts(arriving):
                if _shared(_box(old), _box(new)) > 0.25:
                    report(
                        ("fade", where()),
                        f"Ghosting at {where()}: '{_label(new)}' appears while "
                        f"'{_label(old)}' is still fading out in the same place. "
                        "Fade the old content out first (swap() or new_slide()), or "
                        "use ReplacementTransform/TransformMatchingTex.",
                    )
        result = original_play(self, *args, **kwargs)
        texts = _texts(self.mobjects)
        boxes = [(text, _box(text)) for text in texts]
        half_w, half_h = config.frame_width / 2, config.frame_height / 2
        for text, (x0, y0, x1, y1) in boxes:
            edges = [
                name
                for name, out in (
                    ("left", x0 < -half_w),
                    ("right", x1 > half_w),
                    ("bottom", y0 < -half_h),
                    ("top", y1 > half_h),
                )
                if out
            ]
            if edges:
                report(
                    ("edge", _label(text)),
                    f"'{_label(text)}' runs off the {' and '.join(edges)} edge after "
                    f"{where()}. Scale it down or use fit().",
                )
        for index, (first, a) in enumerate(boxes):
            for second, b in boxes[index + 1 :]:
                if _shared(a, b) > 0.2:
                    report(
                        ("overlap", _label(first), _label(second)),
                        f"'{_label(first)}' and '{_label(second)}' overlap after "
                        f"{where()}. Move one with next_to(..., buff=0.2) or arrange().",
                    )
        return result

    Scene.play = play


def _cues() -> list[str]:
    """Paragraphs whose narrate() block has started, in order."""
    return list(getattr(sys.modules.get("studio_narration"), "_cues", {}))


def snapshot(scene, frames: list, label: str):
    from PIL import Image

    scene.renderer.update_frame(scene, ignore_skipping=True)
    image = Image.fromarray(scene.renderer.get_frame()).convert("RGB")
    image.thumbnail(THUMBNAIL)
    frames.append((scene.renderer.time, label, image))


def watch_frames(frames: list):
    """Keep the frame a viewer sees as each narrate() block ends.

    A block ends where the next one starts, so the frame is taken just before
    the first animation after a new cue. Scripts without narrate() get the
    frame before each slide change (text fading out) instead.
    """
    from manim import FadeOut, Scene

    original_play = Scene.play
    seen = {"cues": 0}

    def play(self, *args, **kwargs):
        cues = _cues()
        if len(cues) > seen["cues"]:
            if seen["cues"]:
                snapshot(self, frames, f"#{cues[seen['cues'] - 1]}")
            seen["cues"] = len(cues)
        elif not cues and _texts(
            arg.mobject for arg in args if isinstance(arg, FadeOut)
        ):
            snapshot(self, frames, "")
        return original_play(self, *args, **kwargs)

    Scene.play = play


def write_preview(frames: list, path: Path) -> int:
    """Lay the frames out in one labelled image; returns how many it shows."""
    from PIL import Image, ImageDraw, ImageFont

    if len(frames) > PREVIEW_FRAMES:
        step = (len(frames) - 1) / (PREVIEW_FRAMES - 1)
        frames = [frames[round(index * step)] for index in range(PREVIEW_FRAMES)]
    width, height = THUMBNAIL
    rows = -(-len(frames) // PREVIEW_COLUMNS)
    columns = min(len(frames), PREVIEW_COLUMNS)
    sheet = Image.new("RGB", (columns * (width + 4) - 4, rows * (height + 4) - 4))
    sheet.paste((128, 128, 128), (0, 0, *sheet.size))
    draw = ImageDraw.Draw(sheet)
    font = ImageFont.load_default(size=15)
    for index, (time, label, image) in enumerate(frames):
        x = index % PREVIEW_COLUMNS * (width + 4)
        y = index // PREVIEW_COLUMNS * (height + 4)
        sheet.paste(image, (x, y))
        # Bottom-left, where slides keep a margin; titles sit at the top.
        caption = f"{label} {int(time) // 60}:{int(time) % 60:02d}".strip()
        corner = (x + 6, y + image.height - 22)
        box = draw.textbbox(corner, caption, font=font)
        draw.rectangle((box[0] - 4, box[1] - 2, box[2] + 4, box[3] + 2), fill="black")
        draw.text(corner, caption, fill="white", font=font)
    path.parent.mkdir(exist_ok=True)
    sheet.save(path)
    return len(frames)


def run_scene(
    script: Path,
    scene_name: str,
    problems: list[str] | None = None,
    frames: list | None = None,
) -> float:
    """Execute construct() with Manim's dry run and return the scene length."""
    from manim import config, tempconfig

    if problems is not None:
        watch_layout(script, problems)
    if frames is not None:
        watch_frames(frames)
    with tempfile.TemporaryDirectory(prefix="studio-check-") as media:
        options = {
            "dry_run": True,
            "renderer": "cairo",
            "media_dir": media,
            "disable_caching": True,
            "verbosity": "ERROR",
            "progress_bar": "none",
            "quality": "low_quality",
        }
        with tempconfig(options):
            config.input_file = str(script)
            spec = importlib.util.spec_from_file_location("studio_check_scene", script)
            module = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = module
            spec.loader.exec_module(module)
            scene = getattr(module, scene_name)()
            scene.render()
            if frames is not None:
                cues = _cues()
                snapshot(scene, frames, f"#{cues[-1]}" if cues else "")
            return float(scene.renderer.time)


def main(argv: list[str]) -> int:
    folder = Path.cwd()
    script = folder / (argv[0] if argv else "script.py")
    problems, notes, frames = [], [], []
    # A stale sheet would show an older version of the scene.
    (folder / PREVIEW).unlink(missing_ok=True)
    try:
        source = script.read_text(encoding="utf-8")
    except OSError as error:
        print(f"FAIL: cannot read {script.name}: {error}")
        return 1
    try:
        result = scenes.analyze(source)
    except scenes.ScriptError as error:
        print(f"FAIL: {error}")
        return 1
    if result["error"]:
        print(f"FAIL: {script.name}: {result['error']}")
        return 1
    scene_name = argv[1] if len(argv) > 1 else result["scenes"][-1]
    if scene_name not in result["scenes"]:
        print(
            f"FAIL: scene {scene_name} not found. Scenes: {', '.join(result['scenes'])}"
        )
        return 1
    if len(result["scenes"]) > 1:
        notes.append(
            f"Several scenes found ({', '.join(result['scenes'])}); checked {scene_name}."
        )
    if result["needs_latex"] and not (
        shutil.which("latex") and shutil.which("dvisvgm")
    ):
        problems.append(
            "Tex/MathTex need LaTeX, which is not installed here. Use MathTypst instead."
        )
    for asset in referenced_assets(ast.parse(result["source"])):
        if not (folder / asset).is_file():
            problems.append(f"Missing asset: {asset}")
    duration = None
    timed = narration.uses_timing(result["source"])
    cues = tempfile.TemporaryDirectory(prefix="studio-cues-")
    if timed:
        # narrate() records where each line starts, as it does during a render.
        os.environ["MANIM_STUDIO_NARRATION_CUES"] = cues.name
        os.environ.pop("MANIM_STUDIO_NARRATION_TIMING", None)
    if not problems:
        sys.path.insert(0, str(folder))
        os.chdir(folder)
        try:
            duration = run_scene(script, scene_name, problems, frames)
        except (
            BaseException
        ) as error:  # Report any scene failure, including SystemExit.
            frames = [
                frame
                for frame in traceback.extract_tb(error.__traceback__)
                if Path(frame.filename).resolve() == script.resolve()
            ]
            where = f" (line {frames[-1].lineno})" if frames else ""
            problems.append(f"{type(error).__name__}{where}: {error}")
            if not frames:
                problems.append(
                    "".join(traceback.format_exception(error)[-6:]).strip()[-2000:]
                )
    try:
        text = (folder / "narration.txt").read_text(encoding="utf-8")
    except OSError:
        text = ""
    if text.strip() and timed:
        try:
            count = len(narration.parse_timeline(text))
        except narration.NarrationError as error:
            problems.append(f"narration.txt: {error}")
        else:
            if duration is not None:
                started = narration.read_cues(cues.name)
                missing = [str(n) for n in range(1, count + 1) if n not in started]
                if missing:
                    problems.append(
                        "Narration paragraph(s) "
                        + ", ".join(missing)
                        + " have no narrate() block. Wrap the animations for each"
                        " paragraph in `with narrate(self, N) as line:`."
                    )
                order = sorted(started, key=started.get)
                if order != sorted(order):
                    notes.append("narrate() blocks run out of paragraph order.")
            notes.append(f"Narration has {count} paragraph(s) timed with narrate().")
    elif text.strip():
        try:
            spoken = narration_end(text)
        except narration.NarrationError as error:
            problems.append(f"narration.txt: {error}")
        else:
            if duration is not None and spoken > duration + 1:
                problems.append(
                    f"Narration needs about {spoken:.0f}s but the animation lasts "
                    f"{duration:.1f}s. Lengthen run_time/self.wait() or shorten the narration."
                )
            notes.append(f"Narration takes about {spoken:.0f}s to speak.")
    if duration is not None:
        notes.insert(0, f"Scene {scene_name} runs {duration:.1f}s.")
    if duration is not None and frames:
        try:
            shown = write_preview(frames, folder / PREVIEW)
        except Exception as error:  # The preview is a convenience, never a failure.
            notes.append(f"No preview image: {error}")
        else:
            moments = "narration paragraph" if timed else "slide"
            more = f" ({shown} of {len(frames)})" if shown < len(frames) else ""
            label = "#paragraph and time" if timed else "time"
            notes.append(
                f"Preview: {PREVIEW.as_posix()} shows the frame at the end of each "
                f"{moments}{more}, labelled with its {label}. View it to check the "
                "layout."
            )
    for note in notes:
        print(f"NOTE: {note}")
    for problem in problems:
        print(f"FAIL: {problem}")
    cues.cleanup()
    if not problems:
        print("OK: the project is ready to render.")
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
