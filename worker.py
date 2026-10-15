"""Manim subprocess entry for both bundled Python and development Python."""

import runpy
import sys
import json
import os
from fractions import Fraction
from pathlib import Path
import traceback

# Studio assigns the process tree before renderer and scene imports.
if os.environ.pop("MANIM_STUDIO_WORKER_GATE", None) == "1":
    if sys.stdin.readline() != "1\n":
        raise SystemExit("Studio could not prepare the render process.")

custom_fps = os.environ.pop("MANIM_STUDIO_CUSTOM_FPS", None)
narration_spec = os.environ.pop("MANIM_STUDIO_NARRATION", None)
# Studio asks long scenes to be rendered by helper processes (see parallel.py).
# A helper is this same script started with a JSON description of its role.
parallel_setting = os.environ.pop("MANIM_STUDIO_PARALLEL", None)
helper = os.environ.pop("MANIM_STUDIO_HELPER", None)
helper = json.loads(helper) if helper else None
if parallel_setting is not None or helper is not None:
    # Every process of one render must build identical scenes, or the helpers'
    # fingerprints would disagree and the render would start over.
    import random

    random.seed(0)
    try:
        import numpy

        numpy.random.seed(0)
    except ImportError:
        pass

from runtime import DATA_ROOT
import parallel

sys.path.insert(0, str(DATA_ROOT))
# Scripts can import helper modules kept beside them in their project folder.
sys.path.insert(0, os.getcwd())
from manim import error_console

if custom_fps is not None:
    from manim.scene import scene_file_writer

    custom_frame_rate = Fraction(custom_fps)
    requested_fps = float(custom_frame_rate)
    original_frame_rate = scene_file_writer.to_av_frame_rate

    def frame_rate_for_custom_render(fps):
        # Manim 0.21 otherwise rejects valid rates such as 17.5 and snaps NTSC.
        # Keep legacy conversion when the scene deliberately changes its rate.
        return custom_frame_rate if fps == requested_fps else original_frame_rate(fps)

    scene_file_writer.to_av_frame_rate = frame_rate_for_custom_render

original_print_exception = error_console.print_exception
script_path = Path(sys.argv[-2]).resolve()


def report_exception(*args, **kwargs):
    """Keep error locations independent of Rich's console-width wrapping."""
    kind, error, trace = sys.exc_info()
    frames = traceback.extract_tb(trace)
    user_frames = [
        frame for frame in frames if Path(frame.filename).resolve() == script_path
    ]
    if user_frames:
        print(
            "STUDIO_ERROR|"
            + json.dumps(
                {"line": user_frames[-1].lineno, "message": f"{kind.__name__}: {error}"}
            ),
            flush=True,
        )
    return original_print_exception(*args, **kwargs)


error_console.print_exception = report_exception


def run_manim():
    try:
        runpy.run_module("manim", run_name="__main__", alter_sys=True)
    except SystemExit as finished:
        if finished.code not in (0, None):
            raise


if helper is not None:
    import atexit
    from manim import config
    from manim.renderer.cairo_renderer import CairoRenderer

    seen = {}
    original_status = CairoRenderer.update_skipping_status
    original_play = CairoRenderer.play

    if helper["role"] == "plan":
        # Skip every animation and record how long each one is.
        played = []

        def planning_status(self):
            original_status(self)
            self.skip_animations = True

        def planning_play(self, scene, *args, **kwargs):
            original_play(self, scene, *args, **kwargs)
            played.append(
                [float(scene.duration), bool(scene.is_current_animation_frozen_frame())]
            )

        CairoRenderer.update_skipping_status = planning_status
        CairoRenderer.play = planning_play
        atexit.register(
            lambda: Path(helper["output"]).write_text(
                json.dumps({"fps": float(config.frame_rate), "animations": played}),
                encoding="utf-8",
            )
        )
    else:
        # Render only this helper's blocks; fingerprint the scene at every block
        # start so the parent can prove that skipping matched real playback.
        from manim.scene import scene_file_writer

        fingerprints = open(helper["output"], "a", encoding="utf-8", buffering=1)

        def sharded_status(self):
            original_status(self)
            if (self.num_plays // helper["block"]) % helper["count"] != helper["index"]:
                self.skip_animations = True

        def sharded_play(self, scene, *args, **kwargs):
            seen["scene"] = scene
            if self.num_plays % helper["block"] == 0:
                fingerprint = parallel.fingerprint(scene, str(config.media_dir))
                print(self.num_plays, fingerprint, file=fingerprints)
            original_play(self, scene, *args, **kwargs)

        def finish_fingerprints():
            scene = seen.get("scene")
            if scene is not None:
                fingerprint = parallel.fingerprint(scene, str(config.media_dir))
                print(f"end_{scene.renderer.num_plays}", fingerprint, file=fingerprints)
            fingerprints.close()

        CairoRenderer.update_skipping_status = sharded_status
        CairoRenderer.play = sharded_play
        # The parent joins every helper's partial movies itself.
        scene_file_writer.SceneFileWriter.combine_to_movie = lambda self: None
        atexit.register(finish_fingerprints)


def run_animation(abort=lambda: None, report=None):
    """Render the animation, with helper processes when the scene is long."""
    if helper is None:
        report = report or (lambda message: None)
        if parallel_setting is not None:
            env = dict(os.environ)
            if custom_fps is not None:
                env["MANIM_STUDIO_CUSTOM_FPS"] = custom_fps
            media = Path(sys.argv[sys.argv.index("--media_dir") + 1])
            try:
                joined = parallel.render(
                    sys.argv,
                    env=env,
                    stage=report,
                    abort=abort,
                    setting=parallel_setting,
                    directory=media.parent / "helpers",
                )
                if joined:
                    return
            except OSError:
                pass  # Rendering without helpers is always possible.
        # Manim's partial-movie cache has no reader here (every render starts in a
        # new folder), so hashing each animation is wasted work.
        sys.argv[1:1] = ["--disable_caching", "--max-inflight-encoders", "3"]
        report(None)  # Manim prints its own progress from here.
    run_manim()


def stage(message):
    print("STUDIO_STAGE|" + message, flush=True)


if narration_spec is None:
    run_animation(report=lambda message: message and stage(message))
else:
    import _thread
    import threading
    import narration

    spec = json.loads(Path(narration_spec).read_text(encoding="utf-8"))
    lines = {"render": None, "voice": None, "quiet": False}
    lock = threading.Lock()

    def show(**changes):
        """One status line for two jobs that now run side by side."""
        with lock:
            lines.update(changes)
            text = " · ".join(
                part for part in (lines["render"], lines["voice"]) if part
            )
            # Once Manim prints its own progress, narration stays out of its way.
            if text and not (lines["quiet"] and lines["render"] is None):
                stage(text)

    def render_report(message):
        if message is None:
            show(render=None, quiet=True)
        else:
            show(render=message)

    def prepare_voice():
        try:
            return narration.prepare_track(
                spec, report=lambda message: show(voice=message)
            )
        except narration.NarrationError:
            raise
        except Exception as error:
            raise narration.NarrationError(
                f"The narration voice failed ({type(error).__name__}: {error})."
            ) from error

    voice = {}

    def narrate():
        try:
            voice["track"] = prepare_voice()
        except BaseException as error:
            voice["error"] = error
            # A bad line must stop the render at once, as it did when the voice
            # was generated first: wake the main thread wherever it is.
            _thread.interrupt_main()

    def voice_failed():
        if "error" in voice:
            raise voice["error"]

    try:
        # The voice is generated while the animation renders. Finished lines are
        # checkpointed, so a retry only generates what changed.
        speaker = threading.Thread(target=narrate, name="narration", daemon=True)
        speaker.start()
        try:
            if not narration.restore_video(spec, report=stage):
                run_animation(abort=voice_failed, report=render_report)
                narration.checkpoint_video(spec)
            # Narration may still be running: show its progress again.
            show(render=None, quiet=False)
            speaker.join()
        except (KeyboardInterrupt, SystemExit):
            # Either the voice failed (and woke us) or the render itself did.
            voice_failed()
            raise
        voice_failed()
        show(voice=None, render=None, quiet=False)
        try:
            narration.narrate_render(
                spec,
                report=stage,
                warn=lambda message: print("STUDIO_WARNING|" + message, flush=True),
                prepared=voice["track"],
            )
        except narration.NarrationError:
            raise
        except Exception as error:
            raise narration.NarrationError(
                f"Adding narration to the video failed ({type(error).__name__}: {error})."
            ) from error
    except narration.NarrationError as error:
        print(f"STUDIO_NARRATION_ERROR|{error}", flush=True)
        raise SystemExit(3)
