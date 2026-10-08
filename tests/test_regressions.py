"""Regression coverage for cancellation, damaged storage and Python formatting."""

import asyncio
import json
import io
from fractions import Fraction
import os
from contextlib import ExitStack
from pathlib import Path
import tempfile
import subprocess
import sys
import threading
import time
import unittest
from unittest.mock import patch
from unittest.mock import MagicMock

import studio
import desktop

SAMPLE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(0.2)\n"


class StorageTests(unittest.TestCase):
    def test_preferences_write_failures_preserve_saved_data_and_allow_retry(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "ROOT", Path(directory)),
        ):
            studio.save_preferences(studio.Preferences(values={"manim-theme": "dark"}))
            for operation in ("write_text", "replace"):
                original = getattr(Path, operation)

                def fail(path, *args, **kwargs):
                    if path.name == "preferences.tmp":
                        raise OSError("disk write failed")
                    return original(path, *args, **kwargs)

                with patch.object(Path, operation, fail):
                    with self.assertRaises(OSError):
                        studio.save_preferences(
                            studio.Preferences(values={"manim-theme": "light"})
                        )
                self.assertEqual(studio.preferences(), {"manim-theme": "dark"})
            studio.save_preferences(studio.Preferences(values={"manim-theme": "light"}))
            self.assertEqual(studio.preferences(), {"manim-theme": "light"})

    def test_corrupt_preferences_recover(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "ROOT", Path(directory)),
        ):
            path = Path(directory) / ".studio/preferences.json"
            path.parent.mkdir()
            for data in (
                "[]",
                "null",
                "{broken",
                '{"manim-theme":3,"unknown":"x"}',
                '{"manim-draft":"\\ud800"}',
                "[" * 20_000 + "0" + "]" * 20_000,
            ):
                path.write_text(data)
                self.assertEqual(studio.preferences(), {})
                studio.save_preferences(
                    studio.Preferences(values={"manim-theme": "dark"})
                )
                self.assertEqual(studio.preferences(), {"manim-theme": "dark"})

    def test_corrupt_manifests_do_not_break_startup(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "OUTPUTS", Path(directory)),
            patch.object(studio, "JOBS", {}),
        ):
            for index, data in enumerate(
                ([], None, {}, {"id": "bad", "name": "broken"}, "text")
            ):
                folder = Path(directory) / str(index)
                folder.mkdir()
                (folder / "video.mp4").write_bytes(b"video")
                (folder / "render.json").write_text(json.dumps(data))

            async def load():
                async with studio.lifespan(studio.app):
                    self.assertEqual(studio.list_renders(), [])

            asyncio.run(load())

    def test_deeply_nested_manifest_and_empty_video_are_ignored(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "OUTPUTS", Path(directory)),
            patch.object(studio, "JOBS", {}),
        ):
            nested = Path(directory) / "nested"
            nested.mkdir()
            (nested / "render.json").write_text("[" * 20_000 + "0" + "]" * 20_000)
            empty = Path(directory) / "empty"
            empty.mkdir()
            (empty / "render.json").write_text(json.dumps(completed_job()))
            (empty / "video.mp4").touch()

            async def load():
                async with studio.lifespan(studio.app):
                    self.assertEqual(studio.JOBS, {})

            asyncio.run(load())

    @unittest.skipUnless(os.name == "nt", "Windows directory junction")
    def test_history_does_not_follow_windows_junction_outside_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            outputs, external = root / "outputs", root / "external"
            outputs.mkdir()
            external.mkdir()
            (external / "render.json").write_text(json.dumps(completed_job()))
            (external / "video.mp4").write_bytes(b"video")
            link = outputs / "History"
            quote = lambda path: "'" + str(path).replace("'", "''") + "'"
            result = subprocess.run(
                [
                    "powershell",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    f"New-Item -ItemType Junction -Path {quote(link)} -Target {quote(external)} -ErrorAction Stop | Out-Null",
                ],
                capture_output=True,
            )
            if result.returncode:
                self.skipTest(
                    f"Junction creation unavailable: {result.stderr.decode(errors='replace')}"
                )
            with (
                patch.object(studio, "OUTPUTS", outputs),
                patch.object(studio, "JOBS", {}),
            ):

                async def load():
                    async with studio.lifespan(studio.app):
                        self.assertEqual(studio.JOBS, {})

                asyncio.run(load())

    def test_lifespan_waits_for_cleanup_on_exceptional_exit(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "OUTPUTS", Path(directory)),
            patch.object(studio, "shutdown_render") as shutdown,
        ):

            async def leave():
                async with studio.lifespan(studio.app):
                    raise RuntimeError("lifespan interrupted")

            with self.assertRaises(RuntimeError):
                asyncio.run(leave())
            shutdown.assert_called_once_with()

    def test_shutdown_waits_for_worker_when_process_kill_is_denied(self):
        process = MagicMock()
        process.poll.return_value = None
        process.kill.side_effect = OSError("kill denied")
        worker = MagicMock()
        worker.is_alive.return_value = True
        job_id = "b" * 32
        with (
            patch.object(studio, "ACTIVE", job_id),
            patch.object(studio, "PROCESS", process),
            patch.object(studio, "WORKER", worker),
            patch.object(studio, "cancel_active", side_effect=OSError("cancel denied")),
            self.assertLogs(level="ERROR") as logs,
        ):
            studio.shutdown_render()
            self.assertEqual(studio.ACTIVE, job_id)
            self.assertIs(studio.PROCESS, process)
        worker.join.assert_called_once_with(timeout=15)
        self.assertTrue(any("Unable to kill" in entry for entry in logs.output))
        self.assertTrue(any("shutdown deadline" in entry for entry in logs.output))

    @unittest.skipUnless(os.name == "nt", "Windows job handle")
    def test_job_close_failure_retains_handle_for_retry(self):
        job = studio.WindowsRenderJob.__new__(studio.WindowsRenderJob)
        job.lock = threading.Lock()
        job.handle = 123
        job.kernel = MagicMock()
        job.kernel.CloseHandle.side_effect = [False, True]
        with self.assertRaises(OSError):
            job.close()
        self.assertEqual(job.handle, 123)
        job.close()
        self.assertIsNone(job.handle)

    def test_process_monitor_diagnoses_tree_failure_and_closes_job(self):
        process = MagicMock()
        with (
            patch.object(
                studio, "stop_process", side_effect=OSError("termination denied")
            ),
            patch.object(studio, "release_process_tree") as release,
            self.assertLogs(level="ERROR"),
        ):
            monitor = studio.start_process_monitor(process)
            monitor.join(timeout=5)
            self.assertFalse(monitor.is_alive())
        release.assert_called_once_with(process)

    def test_format_preserves_string_semantics(self):
        source = (
            'value = """first  \n\tkeep tabs and spaces  \n    last"""\nother=5   \n'
        )
        before = {}
        after = {}
        exec(source, before)
        formatted = studio.format_source(source)
        exec(formatted, after)
        self.assertEqual(before["value"], after["value"])
        self.assertIn("other=5\n", formatted)
        with self.assertRaises(SyntaxError):
            studio.format_source('value = """unfinished')

    def test_invalid_unicode_and_overly_nested_scripts_are_validation_errors(self):
        malformed = 'value="' + chr(0xD800) + '"'
        with self.assertRaises(studio.HTTPException) as error:
            studio.analyze(malformed)
        self.assertEqual(error.exception.status_code, 422)
        nested = "+" * 3000 + "1"
        self.assertIn("deeply nested", studio.analyze(nested)["error"])
        with self.assertRaises(studio.HTTPException) as error:
            studio.format_script(studio.Script(source=nested))
        self.assertEqual(error.exception.status_code, 422)

    def test_manifest_rejects_nonfinite_numbers(self):
        job = {
            "id": "a" * 32,
            "name": "Test",
            "scene": "MainScene",
            "quality": "preview",
            "created": "2026-01-01T00:00:00+00:00",
            "status": "completed",
            "resolution": "480p",
            "video": "/video",
            "size": 1,
            "fps": 15,
            "elapsed": 1,
        }
        self.assertTrue(studio.valid_manifest(job))
        for field in ("size", "fps", "elapsed"):
            for value in (float("nan"), float("inf"), float("-inf"), 10**1000):
                with self.subTest(field=field, value=value):
                    self.assertFalse(studio.valid_manifest({**job, field: value}))

    def test_custom_manifest_dimensions_are_optional_and_validated(self):
        legacy = completed_job()
        self.assertTrue(studio.valid_manifest(legacy))
        self.assertTrue(
            studio.valid_manifest(
                {
                    **legacy,
                    "width": 640,
                    "height": 360,
                    "fps": 17.5,
                    "resolution": "640x360",
                }
            )
        )
        for extra in (
            {"width": 640},
            {"width": True, "height": 360},
            {"width": 640.0, "height": 360},
            {"width": 0, "height": 360},
            {"width": 640, "height": -2},
            {"width": 640, "height": 361},
            {"width": 32768, "height": 2},
        ):
            with self.subTest(extra=extra):
                self.assertFalse(studio.valid_manifest({**legacy, **extra}))

    def test_optional_corrupt_manifest_metadata_is_ignored_at_startup(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "OUTPUTS", Path(directory)),
            patch.object(studio, "JOBS", {}),
        ):
            optional = {
                "legacy_metadata": {"description": "Still compatible", "values": [1, 2]}
            }
            valid = {**completed_job(), **optional}
            self.assertTrue(studio.valid_manifest(valid))
            for index, extra in enumerate(
                (optional, {"stage": float("nan")}, {"unexpected": chr(0xD800)})
            ):
                folder = Path(directory) / str(index)
                folder.mkdir()
                (folder / "video.mp4").write_bytes(b"video")
                (folder / "render.json").write_text(
                    json.dumps({**completed_job(), **extra})
                )
            nested = Path(directory) / "nested"
            nested.mkdir()
            (nested / "video.mp4").write_bytes(b"video")
            (nested / "render.json").write_text(
                json.dumps(completed_job())[:-1]
                + ',"extra":'
                + "[" * 20_000
                + "0"
                + "]" * 20_000
                + "}"
            )

            async def load():
                async with studio.lifespan(studio.app):
                    self.assertEqual(len(studio.JOBS), 1)
                    loaded = studio.JOBS[valid["id"]]
                    self.assertEqual(loaded["folder"], "0")
                    self.assertEqual(
                        loaded["legacy_metadata"], optional["legacy_metadata"]
                    )
                    json.dumps(studio.list_renders(), allow_nan=False).encode("utf-8")

            asyncio.run(load())

    def test_cancel_rejected_after_completion_commit(self):
        job_id = "b" * 32
        with (
            patch.object(studio, "ACTIVE", job_id),
            patch.object(studio, "JOBS", {job_id: {"status": "completed"}}),
        ):
            with self.assertRaises(studio.HTTPException) as error:
                studio.cancel_render(job_id)
            self.assertEqual(error.exception.status_code, 409)


def completed_job():
    return {
        "id": "a" * 32,
        "name": "Test",
        "scene": "MainScene",
        "quality": "preview",
        "created": "2026-01-01T00:00:00+00:00",
        "status": "completed",
        "resolution": "480p",
        "video": "/video",
        "size": 5,
        "fps": 15,
        "elapsed": 1,
    }


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.stack = ExitStack()
        self.addCleanup(self.stack.close)
        directory = self.stack.enter_context(tempfile.TemporaryDirectory())
        self.root = Path(directory)
        for key, value in {
            "ROOT": self.root,
            "OUTPUTS": self.root / "outputs" / "studio",
            "RENDERS": self.root / "renders",
            "JOBS": {},
            "ACTIVE": None,
            "PROCESS": None,
            "WORKER": None,
        }.items():
            self.stack.enter_context(patch.object(studio, key, value))
        self.stack.enter_context(
            patch.object(studio.subprocess, "Popen", side_effect=self.fake_process)
        )
        self.stack.enter_context(patch.object(studio, "attach_process_tree"))
        self.stack.enter_context(
            patch.object(studio, "start_process_monitor", return_value=None)
        )
        self.addCleanup(studio.shutdown_render)

    def fake_process(self, command, **kwargs):
        media = Path(command[command.index("--media_dir") + 1])
        media.mkdir(parents=True)
        (media / "video.mp4").write_bytes(b"video")
        process = unittest.mock.Mock()
        process.stdout = io.StringIO("Rendering test\n")
        process.poll.return_value = 0
        process.wait.return_value = 0
        return process

    def start(self):
        return studio.start_render(
            studio.RenderRequest(source=SAMPLE, scene="MainScene")
        )

    def wait(self):
        worker = studio.WORKER
        if worker is not None:
            worker.join(timeout=5)
            self.assertFalse(worker.is_alive())
        self.assertIsNone(studio.ACTIVE)
        self.assertIsNone(studio.PROCESS)

    def test_thread_launch_failure_rolls_back_render_slot_and_folder(self):
        with patch.object(
            threading.Thread, "start", side_effect=RuntimeError("no thread")
        ):
            with self.assertRaises(studio.HTTPException) as error:
                self.start()
        self.assertEqual(error.exception.status_code, 503)
        self.assertIsNone(studio.ACTIVE)
        self.assertIsNone(studio.WORKER)
        self.assertEqual(studio.JOBS, {})
        self.assertEqual(list(studio.OUTPUTS.iterdir()), [])
        job = self.start()
        self.wait()
        self.assertEqual(studio.get_render(job["id"])["status"], "completed")

    def test_tree_setup_failure_does_not_release_worker_startup_gate(self):
        process = None

        def create(command, **kwargs):
            nonlocal process
            process = self.fake_process(command, **kwargs)
            return process

        with (
            patch.object(studio.subprocess, "Popen", side_effect=create),
            patch.object(
                studio, "attach_process_tree", side_effect=OSError("job setup denied")
            ),
        ):
            job = self.start()
            self.wait()
        process.stdin.write.assert_not_called()
        self.assertEqual(studio.get_render(job["id"])["status"], "failed")
        self.assertIn("job setup denied", studio.get_render(job["id"])["error"])

    def test_legacy_worker_launch_does_not_inherit_reserved_custom_fps(self):
        captured = []

        def create(command, **kwargs):
            captured.append((command, kwargs["env"]))
            return self.fake_process(command, **kwargs)

        with (
            patch.dict(os.environ, {"MANIM_STUDIO_CUSTOM_FPS": "17.5"}),
            patch.object(studio.subprocess, "Popen", side_effect=create),
        ):
            self.start()
            self.wait()
        command, env = captured[0]
        self.assertNotIn("MANIM_STUDIO_CUSTOM_FPS", env)
        self.assertNotIn("--fps", command)
        self.assertNotIn("--resolution", command)

    def test_custom_frame_rate_clock_is_canonical_without_arbitrary_minimum(self):
        for fps in (17.5, 29.97, 7e-6, 1 / 131071):
            with self.subTest(fps=fps):
                request = studio.RenderRequest(
                    source=SAMPLE, scene="MainScene", fps=fps
                )
                self.assertEqual(
                    request.fps, float(Fraction(str(fps)).limit_denominator(1_000_000))
                )

    def test_exact_encoder_representable_frame_rates_are_not_rounded(self):
        for fps in (17.5000001, 1.0000001, 29.9700001):
            with self.subTest(fps=fps):
                request = studio.RenderRequest(
                    source=SAMPLE, scene="MainScene", fps=fps
                )
                self.assertEqual(request.fps, fps)
                self.assertEqual(
                    studio.frame_rate_fraction(request.fps), Fraction(str(fps))
                )

    def test_manifest_publish_failure_preserves_previous_shortcut(self):
        studio.OUTPUTS.mkdir(parents=True)
        shortcut = self.root / "outputs" / "preview.mp4"
        shortcut.write_bytes(b"previous video")
        original = Path.replace

        def replace(path, target):
            if Path(target).name == "render.json":
                raise OSError("disk write failed")
            return original(path, target)

        with patch.object(Path, "replace", replace):
            job = self.start()
            self.wait()
        result = studio.get_render(job["id"])
        self.assertEqual(result["status"], "failed")
        self.assertIn("disk write failed", result["error"])
        self.assertEqual(shortcut.read_bytes(), b"previous video")
        self.assertFalse((studio.OUTPUTS / job["folder"] / "video.mp4").exists())
        self.assertFalse((studio.OUTPUTS / job["folder"] / "render.json").exists())
        self.assertEqual(list(self.root.rglob("*.tmp")), [])
        with self.assertRaises(studio.HTTPException) as error:
            studio.render_file(job["id"], "video")
        self.assertEqual(error.exception.status_code, 404)

    def test_staged_manifest_write_failure_does_not_publish_video(self):
        original = Path.write_text

        def write(path, *args, **kwargs):
            if path.name == "render.tmp":
                raise OSError("manifest disk full")
            return original(path, *args, **kwargs)

        with patch.object(Path, "write_text", write):
            job = self.start()
            self.wait()
        self.assertEqual(studio.get_render(job["id"])["status"], "failed")
        self.assertFalse((studio.OUTPUTS / job["folder"] / "video.mp4").exists())
        self.assertFalse((self.root / "outputs" / "preview.mp4").exists())
        self.assertEqual(list(self.root.rglob("*.tmp")), [])

    def test_shortcut_bulk_copy_does_not_hold_job_lock(self):
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        original = studio.shutil.copy2

        def copy(source, destination, *args, **kwargs):
            if Path(destination).name.startswith(".preview-"):
                entered.set()
                if not release.wait(5):
                    raise TimeoutError("Test did not release copy")
            return original(source, destination, *args, **kwargs)

        with patch.object(studio.shutil, "copy2", side_effect=copy):
            job = self.start()
            self.assertTrue(entered.wait(5))
            before = time.monotonic()
            self.assertEqual(studio.get_render(job["id"])["status"], "rendering")
            self.assertLess(time.monotonic() - before, 0.5)
            studio.cancel_render(job["id"])
            release.set()
            self.wait()
        self.assertEqual(studio.get_render(job["id"])["status"], "cancelled")
        self.assertFalse((self.root / "outputs" / "preview.mp4").exists())
        self.assertEqual(list(self.root.rglob("*.tmp")), [])

    def test_cleanup_finishes_before_terminal_snapshot_and_shutdown(self):
        entered, release = threading.Event(), threading.Event()
        self.addCleanup(release.set)
        original = studio.shutil.rmtree

        def cleanup(path, *args, **kwargs):
            if Path(path).parent == studio.RENDERS:
                entered.set()
                if not release.wait(5):
                    raise TimeoutError("Test did not release cleanup")
            return original(path, *args, **kwargs)

        with patch.object(studio.shutil, "rmtree", side_effect=cleanup):
            job = self.start()
            self.assertTrue(entered.wait(5))
            result = studio.get_render(job["id"])
            self.assertEqual(result["status"], "rendering")
            self.assertFalse(result["can_cancel"])
            self.assertEqual(studio.list_renders()[0]["status"], "rendering")
            with self.assertRaises(studio.HTTPException) as error:
                self.start()
            self.assertEqual(error.exception.status_code, 409)
            shutdown = threading.Thread(target=studio.shutdown_render)
            shutdown.start()
            time.sleep(0.05)
            self.assertTrue(shutdown.is_alive())
            release.set()
            shutdown.join(timeout=5)
            self.assertFalse(shutdown.is_alive())
            self.wait()
        self.assertEqual(studio.get_render(job["id"])["status"], "completed")
        self.start()
        self.wait()

    def test_failure_and_cancellation_stay_active_until_cleanup_finishes(self):
        original = studio.shutil.rmtree
        for outcome in ("failed", "cancelled"):
            with self.subTest(outcome=outcome):
                entered, release = threading.Event(), threading.Event()
                self.addCleanup(release.set)

                def process(command, **kwargs):
                    result = self.fake_process(command, **kwargs)
                    if outcome == "failed":
                        result.wait.return_value = 1
                    else:
                        studio.JOBS[studio.ACTIVE]["_cancel"] = True
                    return result

                def cleanup(path, *args, **kwargs):
                    if Path(path).parent == studio.RENDERS:
                        entered.set()
                        if not release.wait(5):
                            raise TimeoutError("Test did not release cleanup")
                    return original(path, *args, **kwargs)

                with (
                    patch.object(studio.subprocess, "Popen", side_effect=process),
                    patch.object(studio.shutil, "rmtree", side_effect=cleanup),
                ):
                    job = self.start()
                    self.assertTrue(entered.wait(5))
                    snapshot = studio.get_render(job["id"])
                    self.assertEqual(snapshot["status"], "rendering")
                    self.assertFalse(snapshot["can_cancel"])
                    release.set()
                    self.wait()
                self.assertEqual(studio.get_render(job["id"])["status"], outcome)

    def test_cleanup_errors_still_release_exited_process_and_remove_staging(self):
        for operation in ("wait", "close"):
            with self.subTest(operation=operation):

                def process(command, **kwargs):
                    result = self.fake_process(command, **kwargs)
                    if operation == "wait":
                        result.wait.side_effect = [0, OSError("wait failed")]
                    else:
                        output = MagicMock()
                        output.__iter__.return_value = iter(result.stdout)
                        output.close.side_effect = OSError("close failed")
                        result.stdout = output
                    return result

                with (
                    patch.object(studio.subprocess, "Popen", side_effect=process),
                    self.assertLogs(level="ERROR"),
                ):
                    job = self.start()
                    self.wait()
                self.assertEqual(studio.get_render(job["id"])["status"], "completed")
                self.assertEqual(list(self.root.rglob("*.tmp")), [])
                self.assertFalse((studio.RENDERS / job["id"]).exists())

    def test_live_process_that_cannot_be_killed_keeps_render_slot_reserved(self):
        live = None

        def process(command, **kwargs):
            nonlocal live
            live = self.fake_process(command, **kwargs)
            output = MagicMock()
            output.__iter__.side_effect = OSError("output interrupted")
            live.stdout = output
            live.poll.return_value = None
            live.kill.side_effect = OSError("kill denied")
            return live

        with (
            patch.object(studio.subprocess, "Popen", side_effect=process),
            patch.object(studio, "stop_process", side_effect=OSError("stop denied")),
            self.assertLogs(level="ERROR"),
        ):
            job = self.start()
            worker = studio.WORKER
            worker.join(timeout=5)
            self.assertFalse(worker.is_alive())
        self.assertEqual(studio.ACTIVE, job["id"])
        self.assertIs(studio.PROCESS, live)
        with self.assertRaises(studio.HTTPException) as error:
            self.start()
        self.assertEqual(error.exception.status_code, 409)
        # The fake OS process is now gone; restore globals before fixture teardown.
        live.poll.return_value = 0
        studio.ACTIVE, studio.PROCESS = None, None

    def test_output_files_cannot_follow_links_outside_render_folder(self):
        job = self.start()
        self.wait()
        external = self.root / "external.txt"
        external.write_text("private data")
        folder = studio.OUTPUTS / job["folder"]
        for kind, filename in (
            ("video", "video.mp4"),
            ("script", "script.py"),
            ("log", "render.log"),
        ):
            path = folder / filename
            path.unlink()
            try:
                path.symlink_to(external)
            except OSError as error:
                self.skipTest(f"Symlink creation unavailable: {error}")
            with self.assertRaises(studio.HTTPException) as error:
                studio.render_file(job["id"], kind)
            self.assertEqual(error.exception.status_code, 404)
        self.assertEqual(studio.get_render(job["id"])["log"], "")


class DesktopTests(unittest.TestCase):
    def test_native_close_waits_for_render_shutdown(self):
        with tempfile.TemporaryDirectory() as directory:
            webview = MagicMock()
            webview.settings = {}
            server = MagicMock()
            server.started = True
            with (
                patch.dict(sys.modules, {"webview": webview}),
                patch.object(sys, "argv", ["desktop.py", "--port", "0"]),
                patch.object(desktop, "DATA_ROOT", Path(directory)),
                patch.object(desktop, "PACKAGE_ROOT", Path(directory)),
                patch.object(desktop.uvicorn, "Server", return_value=server),
                patch.object(desktop.logging, "basicConfig"),
                patch.object(desktop, "shutdown_render") as shutdown,
            ):
                desktop.main()
            shutdown.assert_called_once_with()
            self.assertTrue(server.should_exit)

    def test_native_server_stops_even_when_render_shutdown_raises(self):
        with tempfile.TemporaryDirectory() as directory:
            webview = MagicMock()
            webview.settings = {}
            server = MagicMock()
            server.started = True
            thread = MagicMock()
            with (
                patch.dict(sys.modules, {"webview": webview}),
                patch.object(sys, "argv", ["desktop.py", "--port", "0"]),
                patch.object(desktop, "DATA_ROOT", Path(directory)),
                patch.object(desktop, "PACKAGE_ROOT", Path(directory)),
                patch.object(desktop.uvicorn, "Server", return_value=server),
                patch.object(desktop.threading, "Thread", return_value=thread),
                patch.object(desktop.logging, "basicConfig"),
                patch.object(
                    desktop, "shutdown_render", side_effect=OSError("shutdown denied")
                ),
            ):
                with self.assertRaises(OSError):
                    desktop.main()
            self.assertTrue(server.should_exit)
            thread.join.assert_called_once_with(timeout=5)


if __name__ == "__main__":
    unittest.main()
