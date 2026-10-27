"""Queue ordering, cancellation and PDF project isolation."""

import asyncio
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

from fastapi import HTTPException
from starlette.requests import Request

import projects
import studio

SOURCE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(0.1)\n"


class RenderQueueTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        for name, value in {
            "JOBS": {},
            "ACTIVE": None,
            "WORKER": None,
            "PROCESS": None,
            "OUTPUTS": Path(self.directory.name),
            "UPDATE_INSTALLING": False,
            "QUEUE_PAUSED": None,
        }.items():
            context = patch.object(studio, name, value)
            context.start()
            self.addCleanup(context.stop)
        context = patch.object(studio.threading, "Thread")
        self.thread = context.start()
        self.addCleanup(context.stop)
        context = patch.dict(
            studio.AGENTS.settings, {"queue_start": "auto", "queue_on_error": "fix"}
        )
        context.start()
        self.addCleanup(context.stop)

    def submit(self, **changes):
        return studio.start_render(
            studio.RenderRequest(
                **{"source": SOURCE, "scene": "MainScene", "queue": True, **changes}
            )
        )

    def test_fifo_and_cancel_waiting_job_without_stopping_active(self):
        first = self.submit(name="First")
        second = self.submit(name="Second")
        third = self.submit(name="Third")
        self.assertEqual(studio.ACTIVE, first["id"])
        self.assertEqual(self.thread.call_count, 1)
        self.assertNotIn("_source", second)
        with patch.object(studio, "stop_process") as stop:
            studio.cancel_render(second["id"])
            stop.assert_not_called()
        self.assertNotIn("_source", studio.JOBS[second["id"]])
        studio.JOBS[first["id"]]["status"] = "completed"
        studio.ACTIVE = None
        with studio.LOCK:
            studio.start_next_render()
        self.assertEqual(studio.ACTIVE, third["id"])
        self.assertEqual(
            self.thread.call_args.kwargs["args"],
            (third["id"], studio.analyze(SOURCE)["source"]),
        )

    def finish_active(self, status="completed"):
        studio.JOBS[studio.ACTIVE]["status"] = status
        studio.ACTIVE = None
        with studio.LOCK:
            studio.start_next_render()

    def test_manual_start_holds_videos_until_the_queue_is_started(self):
        studio.AGENTS.settings["queue_start"] = "manual"
        first = self.submit(name="First")
        second = self.submit(name="Second")
        self.assertIsNone(studio.ACTIVE)
        self.thread.assert_not_called()
        self.assertEqual(first["stage"], "Waiting for you to start the queue")
        state = studio.queue_state()
        self.assertTrue(state["paused"])
        self.assertEqual(state["waiting"], [first["id"], second["id"]])

        state = studio.start_queue()
        self.assertEqual(studio.ACTIVE, first["id"])
        self.assertFalse(state["paused"])
        self.finish_active()
        self.assertEqual(studio.ACTIVE, second["id"])
        self.finish_active()
        studio.settle_queue()
        # Drained: new videos wait for the next explicit start again.
        self.assertTrue(studio.queue_paused())
        third = self.submit(name="Third")
        self.assertIsNone(studio.ACTIVE)
        self.assertEqual(studio.queue_state()["waiting"], [third["id"]])

    def test_pause_lets_the_active_video_finish_then_holds_the_rest(self):
        first = self.submit(name="First")
        second = self.submit(name="Second")
        self.assertEqual(studio.ACTIVE, first["id"])
        self.assertTrue(studio.pause_queue()["paused"])
        self.finish_active()
        self.assertIsNone(studio.ACTIVE)
        self.assertEqual(studio.JOBS[second["id"]]["status"], "queued")
        studio.start_queue()
        self.assertEqual(studio.ACTIVE, second["id"])

    def test_switching_to_automatic_start_releases_held_videos(self):
        studio.AGENTS.settings["queue_start"] = "manual"
        first = self.submit(name="First")
        self.assertIsNone(studio.ACTIVE)
        studio.AGENTS.settings["queue_start"] = "auto"
        with studio.LOCK:
            studio.start_next_render()
        self.assertEqual(studio.ACTIVE, first["id"])

    def test_waiting_videos_can_be_reordered_and_run_in_that_order(self):
        active = self.submit(name="Active")
        first, second, third = (self.submit(name=name) for name in ("A", "B", "C"))
        waiting = lambda: studio.queue_state()["waiting"]  # noqa: E731
        self.assertEqual(waiting(), [first["id"], second["id"], third["id"]])
        studio.move_render(third["id"], studio.MoveRequest(to="front"))
        self.assertEqual(waiting(), [third["id"], first["id"], second["id"]])
        studio.move_render(third["id"], studio.MoveRequest(to="down"))
        self.assertEqual(waiting(), [first["id"], third["id"], second["id"]])
        studio.move_render(first["id"], studio.MoveRequest(to="back"))
        self.assertEqual(waiting(), [third["id"], second["id"], first["id"]])
        with self.assertRaises(HTTPException) as error:
            studio.move_render(active["id"], studio.MoveRequest(to="up"))
        self.assertEqual(error.exception.status_code, 409)
        self.finish_active()
        self.assertEqual(studio.ACTIVE, third["id"])

    def test_clear_cancels_waiting_videos_but_not_the_active_one(self):
        active = self.submit(name="Active")
        waiting = [self.submit(name="One"), self.submit(name="Two")]
        with patch.object(studio, "stop_process") as stop:
            state = studio.clear_queue()
            stop.assert_not_called()
        self.assertEqual(state["waiting"], [])
        self.assertEqual(studio.ACTIVE, active["id"])
        for job in waiting:
            self.assertEqual(studio.JOBS[job["id"]]["status"], "cancelled")

    def test_queue_preferences_are_validated(self):
        with self.assertRaises(ValueError):
            studio.AgentSettings(queue_start="whenever")
        with self.assertRaises(ValueError):
            studio.AgentSettings(queue_on_error="explode")
        self.assertEqual(
            studio.AgentSettings(queue_on_error="skip").queue_on_error, "skip"
        )

    def test_failed_worker_start_does_not_block_next_video(self):
        first = self.submit()
        second = self.submit()
        third = self.submit()
        studio.JOBS[first["id"]]["status"] = "failed"
        studio.ACTIVE = None
        failing, ready = MagicMock(), MagicMock()
        failing.start.side_effect = RuntimeError("no thread")
        self.thread.side_effect = [failing, ready]
        with studio.LOCK:
            studio.start_next_render()
        self.assertEqual(studio.JOBS[second["id"]]["status"], "failed")
        self.assertEqual(studio.ACTIVE, third["id"])
        ready.start.assert_called_once()

    def test_shutdown_discards_waiting_work_and_legacy_busy_guard_remains(self):
        self.submit()
        second = self.submit()
        with self.assertRaises(HTTPException) as error:
            self.submit(queue=False)
        self.assertEqual(error.exception.status_code, 409)
        with patch.object(studio, "stop_process"):
            self.thread.return_value.is_alive.return_value = False
            studio.shutdown_render()
        self.assertEqual(studio.JOBS[second["id"]]["status"], "cancelled")
        self.assertNotIn("_source", studio.JOBS[second["id"]])

    def test_queued_project_cannot_be_deleted_before_rendering(self):
        studio.JOBS["waiting"] = {
            "id": "waiting",
            "project": "video",
            "status": "queued",
        }
        with self.assertRaises(HTTPException) as error:
            studio.delete_project("video")
        self.assertEqual(error.exception.status_code, 409)


class PdfVideoTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        context = patch.object(studio, "ROOT", Path(self.directory.name))
        context.start()
        self.addCleanup(context.stop)

    def upload(self, content, name="Video.pdf", project=None):
        async def receive():
            return {"type": "http.request", "body": content, "more_body": False}

        request = Request(
            {"type": "http", "headers": [(b"x-filename", name.encode())]}, receive
        )
        return asyncio.run(studio.project_from_pdf(request, project=project))

    def test_shared_parent_keeps_each_video_and_existing_files_independent(self):
        parent = projects.create(studio.project_root(), "Lessons", SOURCE)
        data = (Path(__file__).parent / "fixtures/documents/reference.pdf").read_bytes()
        first = self.upload(data, "One.pdf", parent["id"])
        second = self.upload(data, "Two.pdf", parent["id"])
        self.assertEqual(first["id"], second["id"])
        self.assertNotEqual(first["workspace"], second["workspace"])
        projects.update(
            studio.project_root(),
            parent["id"],
            source="first script",
            narration={"enabled": True, "text": "First narrator"},
            workspace=first["workspace"],
        )
        projects.update(
            studio.project_root(),
            parent["id"],
            source="second script",
            workspace=second["workspace"],
        )
        self.assertEqual(
            projects.read(studio.project_root(), parent["id"])["source"], SOURCE
        )
        self.assertEqual(
            projects.read(studio.project_root(), parent["id"], first["workspace"])[
                "source"
            ],
            "first script",
        )
        self.assertEqual(
            projects.read(studio.project_root(), parent["id"], first["workspace"])[
                "narration"
            ]["text"],
            "First narrator",
        )
        self.assertEqual(
            projects.read(studio.project_root(), parent["id"], second["workspace"])[
                "source"
            ],
            "second script",
        )
        self.assertEqual(len(projects.list_projects(studio.project_root())), 1)
        for invalid in ("../outside", "0" * 32):
            with self.assertRaises(KeyError):
                projects.read(studio.project_root(), parent["id"], invalid)

    def test_each_pdf_gets_isolated_original_and_extracted_text(self):
        data = (Path(__file__).parent / "fixtures/documents/reference.pdf").read_bytes()
        first, second = self.upload(data), self.upload(data)
        self.assertNotEqual(first["id"], second["id"])
        for project in (first, second):
            assets = projects.assets_dir(studio.project_root(), project["id"])
            self.assertEqual((assets / "video-script.pdf").read_bytes(), data)
            self.assertIn("Page 1", (assets / "video-script.txt").read_text("utf-8"))
            self.assertTrue(project["narration"]["enabled"])

    def test_invalid_or_unreadable_pdf_does_not_create_a_project(self):
        for data, name in ((b"not a pdf", "Video.pdf"), (b"ignored", "script.txt")):
            with self.subTest(name=name), self.assertRaises(HTTPException) as error:
                self.upload(data, name)
            self.assertEqual(error.exception.status_code, 422)
        self.assertEqual(projects.list_projects(studio.project_root()), [])


RICH_LOG = """Manim Community v0.21.0

Animation 0: Create(Circle):  40%|####      | 6/15 [00:00<00:00, 50it/s]
[10/27/26 14:40:22] INFO     Animation 0 : Partial     scene_file_writer.py:192
                             movie file written in
                             'C:\\media\\partial_movie_files\\Intro\\1.mp4'
╭──────────────────── Traceback (most recent call last) ────────────────────╮
│ C:\\app\\.venv\\Lib\\site-packages\\manim\\cli\\render\\commands.py:125 in render │
│                                                                             │
│   124 │   │   │   │   │   scene = SceneClass()                              │
│ ❱ 125 │   │   │   │   │   scene.render()                                    │
│                                                                             │
│ C:\\data\\projects\\Demo\\script.py:8 in construct                        │
│                                                                             │
│    7 │   │   c = Circle()                                                   │
│ ❱  8 │   │   self.play(Transform(c, 5))                                     │
│                                                                             │
│ C:\\app\\.venv\\Lib\\site-packages\\manim\\scene\\scene.py:1207 in play       │
│                                                                             │
│   1206 │   │   start_time = self.time                                      │
│ ❱ 1207 │   │   self.renderer.play(self, *args, **kwargs)                    │
│                                                                             │
│ C:\\app\\.venv\\Lib\\sit                                                  │
│ e-packages\\manim\\animation\\transform.py:205 in begin                     │
│                                                                             │
│   204 │   │   self.target_mobject = self.create_target()                   │
│ ❱ 205 │   │   self.target_copy = self.target_mobject.copy()                │
│   206 │   │   # Note, this potentially changes the structure               │
╰─────────────────────────────────────────────────────────────────────────────╯
AttributeError: 'int' object has no attribute 'copy'
"""


class DiagnosticsTests(unittest.TestCase):
    def test_render_logs_keep_the_project_frames_and_the_error(self):
        from video_workflow import compact_diagnostics

        text = compact_diagnostics("Render error: Manim failed\n" + RICH_LOG * 3)
        self.assertIn("script.py:8 in construct", text)
        self.assertIn("self.play(Transform(c, 5))", text)
        self.assertIn("transform.py:205 in begin", text)  # The frame that raised.
        self.assertIn("self.target_mobject.copy()", text)
        self.assertIn("AttributeError: 'int' object has no attribute 'copy'", text)
        for noise in ("commands.py", "scene.py:1207", "Partial", "40%", "# Note"):
            self.assertNotIn(noise, text)
        self.assertIn("library frame(s) omitted", text)
        self.assertLess(len(text), len(RICH_LOG))

    def test_plain_tracebacks_and_long_logs_are_capped(self):
        from video_workflow import DIAGNOSTIC_LIMIT, compact_diagnostics

        plain = (
            "Traceback (most recent call last):\n"
            '  File "C:\\data\\Demo\\script.py", line 4, in construct\n'
            "    self.play(Write(MathTex('x')))\n"
            '  File "C:\\py\\Lib\\site-packages\\manim\\tex.py", line 9, in run\n'
            "    raise ValueError('latex failed')\n"
            "ValueError: latex failed\n"
        )
        text = compact_diagnostics(plain)
        self.assertIn('script.py", line 4', text)
        self.assertIn("raise ValueError('latex failed')", text)
        self.assertIn("ValueError: latex failed", text)
        long = "\n".join(f"line {number} of output" for number in range(5000))
        capped = compact_diagnostics("Render error: first\n" + long)
        self.assertLessEqual(len(capped), DIAGNOSTIC_LIMIT + 40)
        self.assertTrue(capped.startswith("Render error: first"))
        self.assertTrue(capped.endswith("line 4999 of output"))
