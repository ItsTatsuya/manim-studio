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

from runtime import DATA_ROOT

sys.path.insert(0, str(DATA_ROOT))
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
runpy.run_module("manim", run_name="__main__", alter_sys=True)
