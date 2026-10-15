"""Multi-process rendering: planning gates, fingerprints and fallbacks."""

import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch

import parallel
from runtime import APP_ROOT

LONG = textwrap.dedent(
    """
    from manim import *

    class Demo(Scene):
        def construct(self):
            shapes = VGroup(*[Square(side_length=0.4).shift(RIGHT * i * 0.5) for i in range(6)])
            shapes.move_to(ORIGIN)
            self.add(shapes)
            for index in range(14):
                self.play(shapes[index % 6].animate.shift(UP * 0.3).set_color(BLUE), run_time=0.4)
                self.wait(0.2)
    """
)
NONDETERMINISTIC = LONG.replace(
    "shapes.move_to(ORIGIN)",
    "import os\n        shapes.move_to(ORIGIN).shift(UP * os.getpid() * 1e-6)",
)


def frames(path):
    import av

    with av.open(str(path)) as movie:
        return [
            hashlib.md5(frame.to_ndarray(format="rgb24").tobytes()).hexdigest()
            for frame in movie.decode(video=0)
        ]


class ScriptInspectionTests(unittest.TestCase):
    def inspect(self, source):
        with tempfile.TemporaryDirectory() as directory:
            script = Path(directory) / "scene.py"
            script.write_text(source, encoding="utf-8")
            return parallel.inspect_script(script)

    def test_loops_and_many_plays_are_worth_splitting(self):
        self.assertEqual(self.inspect(LONG), (True, False))
        many = "class S(Scene):\n    def construct(self):\n" + "".join(
            "        self.wait(1)\n" for _ in range(12)
        )
        self.assertEqual(self.inspect(many), (True, False))

    def test_short_scripts_are_left_alone(self):
        source = "class S(Scene):\n    def construct(self):\n        self.wait(1)\n"
        self.assertEqual(self.inspect(source), (False, False))

    def test_features_joined_outside_partial_movies_are_never_split(self):
        for line in (
            "self.add_sound('a.wav')",
            "self.next_section()",
            "self.add_subcaption('x')",
            "self.play(Create(Circle()), subcaption='x')",
        ):
            source = LONG.replace(
                "self.add(shapes)", line + "\n        self.add(shapes)"
            )
            self.assertEqual(self.inspect(source), (False, False), line)

    def test_latex_is_detected(self):
        source = LONG.replace("self.add(shapes)", "self.add(MathTex('x'))")
        self.assertEqual(self.inspect(source), (True, True))

    def test_unreadable_script_is_not_split(self):
        self.assertEqual(self.inspect("def broken(:"), (False, False))


class OptionTests(unittest.TestCase):
    def test_with_options_replaces_values_without_touching_the_rest(self):
        argv = ["-qm", "--media_dir", "a", "--progress_bar", "display", "x.py", "S"]
        self.assertEqual(
            parallel.with_options(argv, media_dir="b", progress_bar="none"),
            ["-qm", "--media_dir", "b", "--progress_bar", "none", "x.py", "S"],
        )

    def test_worker_count_is_bounded(self):
        self.assertEqual(parallel.worker_count("1"), 1)
        self.assertEqual(parallel.worker_count("3"), 3)
        self.assertEqual(parallel.worker_count("64"), parallel.MAX_WORKERS)
        self.assertEqual(parallel.worker_count("junk"), 1)
        self.assertGreaterEqual(parallel.worker_count("auto"), 1)
        with (
            patch.object(parallel, "available_megabytes", return_value=2000),
            patch("os.cpu_count", return_value=32),
        ):
            self.assertEqual(parallel.worker_count("auto"), 1)


class FingerprintTests(unittest.TestCase):
    def scene(self, directory, *, shift=0.0, tag=""):
        from manim import Circle, Scene, config

        config.media_dir = directory
        scene = Scene()
        circle = Circle().shift([shift, 0, 0])
        scene.add(circle)
        # A process-specific attribute must not matter.
        circle.original_id = tag
        return scene

    def test_fingerprint_follows_the_scene_not_the_process(self):
        with tempfile.TemporaryDirectory() as directory:
            media = str(Path(directory) / "media")
            first = parallel.fingerprint(self.scene(media, tag="1"), media)
            same = parallel.fingerprint(self.scene(media, tag="2"), media)
            moved = parallel.fingerprint(self.scene(media, shift=0.5), media)
        self.assertEqual(first, same)
        self.assertNotEqual(first, moved)


class RenderTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        self.env = dict(
            os.environ,
            PYTHONPATH=str(APP_ROOT),
            PYTHONHASHSEED="0",
            PYTHONIOENCODING="utf-8",
            PYTHONUTF8="1",
        )

    def arguments(self, name, script):
        media = self.root / name
        return media, [
            str(APP_ROOT / "worker.py"),
            "-ql",
            "--renderer",
            "cairo",
            "--format",
            "mp4",
            "--progress_bar",
            "none",
            "--silent",
            "--media_dir",
            str(media),
            "-o",
            "video",
            str(script),
            "Demo",
        ]

    def script(self, source):
        path = self.root / "scene.py"
        path.write_text(source, encoding="utf-8")
        return path

    def test_helpers_produce_the_same_frames_as_one_process(self):
        script = self.script(LONG)
        serial_media, argv = self.arguments("serial", script)
        subprocess.run(
            [sys.executable, "-u", *argv[:1], *argv[1:], "--disable_caching"],
            env=self.env,
            check=True,
            capture_output=True,
        )
        shared_media, argv = self.arguments("shared", script)
        stages = []
        with patch.object(parallel, "MIN_WORK", 0):
            joined = parallel.render(
                argv,
                env=self.env,
                stage=stages.append,
                abort=lambda: None,
                setting="2",
                directory=self.root / "helpers",
            )
        self.assertTrue(joined)
        self.assertFalse((self.root / "helpers").exists())
        serial = next(serial_media.rglob("video.mp4"))
        shared = next(shared_media.rglob("video.mp4"))
        self.assertEqual(frames(shared), frames(serial))
        self.assertTrue(any("parallel" in message for message in stages))

    def test_disagreeing_helpers_fall_back_without_writing_a_movie(self):
        script = self.script(NONDETERMINISTIC)
        media, argv = self.arguments("diverging", script)
        with patch.object(parallel, "MIN_WORK", 0):
            joined = parallel.render(
                argv,
                env=self.env,
                stage=lambda message: None,
                abort=lambda: None,
                setting="2",
                directory=self.root / "helpers",
            )
        self.assertFalse(joined)
        self.assertEqual(list(media.rglob("video.mp4")), [])
        self.assertFalse((self.root / "helpers").exists())

    def test_small_scenes_are_declined_before_any_process_starts(self):
        script = self.script(LONG)
        media, argv = self.arguments("small", script)
        with patch.object(parallel, "_launch") as launch:
            joined = parallel.render(
                argv,
                env=self.env,
                stage=lambda message: None,
                abort=lambda: None,
                setting="1",
                directory=self.root / "helpers",
            )
        self.assertFalse(joined)
        launch.assert_not_called()


if __name__ == "__main__":
    unittest.main()
