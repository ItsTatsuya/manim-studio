"""Video recovery preserves chat/workspace scope and stops instead of looping."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import agents
import projects
import scenes
from video_workflow import MAX_REPAIRS, VideoProblem, VideoWorkflows

SOURCE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(.1)\n"


class VideoRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.project = projects.create(self.root / "projects", "Recovery", SOURCE)
        self.manager = agents.AgentManager(
            self.root, python="python", app_root=self.root
        )
        self.thread = self.manager.create(self.project["id"], provider="codex")
        self.manager.send = Mock(
            side_effect=lambda *a, **kw: {"id": f"turn-{self.manager.send.call_count}"}
        )
        self.submit = Mock(
            side_effect=lambda value: {
                "id": f"render-{self.submit.call_count}",
                "workflow": value["workflow"],
            }
        )
        self.cancel_render = Mock()
        self.flow = VideoWorkflows(
            manager=lambda: self.manager,
            read=lambda project, workspace: projects.read(
                self.root / "projects", project, workspace
            ),
            prepare=self.prepare,
            submit=self.submit,
            render=lambda render: {"log": "ValueError: important traceback detail"},
            cancel_render=self.cancel_render,
            publish=Mock(),
        )

    @staticmethod
    def prepare(project, options, automatic):
        result = scenes.analyze(project["source"])
        if result["error"]:
            raise VideoProblem(result["error"])
        return {"source": result["source"], "scene": result["scenes"][-1], **options}

    def start(self, **kwargs):
        return self.flow.start(self.project["id"], thread=self.thread["id"], **kwargs)

    def finish_turn(self, job, *, status="completed", changed=True):
        turn = {
            "id": f"turn-{self.manager.send.call_count}",
            "status": status,
            "changed": ["script.py"],
            "error": "Usage limit reached",
        }
        if changed:
            projects.update(
                self.root / "projects",
                self.project["id"],
                source=SOURCE + f"\n# repair {self.manager.send.call_count}\n",
            )
        self.flow.agent_finished(self.thread, turn)
        return turn

    def failure(self, job, **changes):
        render = {
            "id": self.flow.jobs[job["id"]]["render"],
            "workflow": job["id"],
            "status": "failed",
            "repairable": True,
            "error": "Manim failed",
            **changes,
        }
        self.flow.render_finished(render)
        return render

    def test_runtime_error_sends_traceback_to_same_chat_and_retries_same_output(self):
        options = {"quality": "final", "width": 1080, "height": 1920, "fps": 24}
        job = self.start(options=options)
        failed = self.failure(job)
        self.manager.send.assert_called_once()
        self.assertEqual(
            self.manager.send.call_args.args[:2],
            (self.project["id"], self.thread["id"]),
        )
        self.assertIn(
            "important traceback detail", self.manager.send.call_args.kwargs["context"]
        )
        self.flow.render_finished(failed)
        self.assertEqual(self.manager.send.call_count, 1)
        turn = self.finish_turn(job)
        self.assertEqual(self.submit.call_count, 2)
        for key, value in options.items():
            self.assertEqual(self.submit.call_args.args[0][key], value)
        self.flow.agent_finished(self.thread, turn)
        self.assertEqual(self.submit.call_count, 2)
        self.flow.render_finished(
            {"id": "render-2", "workflow": job["id"], "status": "completed"}
        )
        self.assertEqual(self.flow.jobs[job["id"]]["status"], "completed")

    def test_syntax_error_is_repaired_before_any_render(self):
        projects.update(
            self.root / "projects",
            self.project["id"],
            source="class Broken(Scene)\n    pass",
        )
        job = self.start()
        self.submit.assert_not_called()
        self.assertEqual(job["status"], "repairing")
        self.assertIn("Line 1", self.manager.send.call_args.kwargs["context"])
        self.finish_turn(job)
        self.assertEqual(self.submit.call_count, 1)

    def test_repair_waits_for_a_cli_update_instead_of_failing(self):
        updating = [True]

        def send(*args, **kwargs):
            self.assertEqual(kwargs["update_wait"], 0)
            if updating[0]:
                raise agents.AgentUpdating("Codex is updating.")
            return {"id": "turn-1"}

        self.manager.send = Mock(side_effect=send)
        with patch("video_workflow.threading.Timer") as timer:
            job = self.start()
            self.failure(job)
            state = self.flow.jobs[job["id"]]
            self.assertEqual(state["status"], "repairing")
            self.assertEqual(state["attempt"], 0)
            self.assertIn("update", state["stage"])
            callback, args = timer.call_args.args[1:]
            updating[0] = False
            callback(*args)
        self.assertEqual(state["attempt"], 1)
        self.assertIn("turn-1", state["_turns"])

    def test_repeated_errors_have_a_finite_repair_budget(self):
        job = self.start()
        for _ in range(MAX_REPAIRS):
            self.failure(job)
            self.finish_turn(job)
        self.failure(job)
        self.assertEqual(self.manager.send.call_count, MAX_REPAIRS)
        self.assertEqual(self.flow.jobs[job["id"]]["status"], "failed")
        self.assertIn("3 attempts", self.flow.jobs[job["id"]]["error"])

    def test_skip_policy_marks_the_video_failed_and_lets_the_queue_continue(self):
        self.manager.settings["queue_on_error"] = "skip"
        self.flow.pause_queue = Mock()
        job = self.start()
        self.failure(job)
        self.manager.send.assert_not_called()
        self.flow.pause_queue.assert_not_called()
        done = self.flow.jobs[job["id"]]
        self.assertEqual(done["status"], "failed")
        self.assertEqual(done["stage"], "Skipped after an error")
        self.assertFalse(self.flow.pending())

    def test_pause_policy_stops_the_queue_after_the_failure(self):
        self.manager.settings["queue_on_error"] = "pause"
        self.flow.pause_queue = Mock()
        job = self.start()
        self.failure(job)
        self.manager.send.assert_not_called()
        self.flow.pause_queue.assert_called_once_with()
        self.assertEqual(
            self.flow.jobs[job["id"]]["stage"], "Queue paused after an error"
        )

    def test_fix_policy_keeps_the_video_pending_while_it_is_repaired(self):
        job = self.start()
        self.failure(job)
        self.assertTrue(self.flow.pending())
        self.assertEqual(self.flow.jobs[job["id"]]["status"], "repairing")

    def test_skip_policy_does_not_repair_a_script_that_fails_the_check(self):
        self.manager.settings["queue_on_error"] = "skip"
        projects.update(
            self.root / "projects",
            self.project["id"],
            source="class Broken(Scene)\n    pass",
        )
        job = self.start()
        self.assertEqual(job["status"], "failed")
        self.manager.send.assert_not_called()

    def test_explicit_send_to_llm_repairs_regardless_of_the_policy(self):
        self.manager.settings["queue_on_error"] = "skip"
        job = self.start(diagnostics="Render error: boom")
        self.assertEqual(job["status"], "repairing")
        self.manager.send.assert_called_once()

    def test_cancelled_repair_does_not_restart_from_a_late_completion(self):
        job = self.start()
        self.failure(job)
        self.manager.interrupt = Mock()
        self.flow.cancel(job["id"])
        self.finish_turn(job)
        self.assertEqual(self.submit.call_count, 1)
        self.manager.interrupt.assert_called_once_with(
            self.project["id"], self.thread["id"]
        )
        self.assertEqual(self.flow.jobs[job["id"]]["status"], "cancelled")

    def test_cancelled_render_and_infrastructure_error_do_not_call_agent(self):
        for changes in ({"status": "cancelled"}, {"repairable": False}):
            job = self.start()
            self.failure(job, **changes)
            self.manager.send.assert_not_called()

    def test_provider_failure_and_no_changes_stop_cleanly(self):
        for status in ("failed", "interrupted", "completed"):
            job = self.start()
            self.failure(job)
            self.finish_turn(job, status=status, changed=False)
            self.assertIn(self.flow.jobs[job["id"]]["status"], {"failed", "cancelled"})
        self.assertEqual(self.submit.call_count, 3)

    def test_new_user_edits_are_not_overwritten_by_an_old_render_error(self):
        job = self.start()
        projects.update(
            self.root / "projects", self.project["id"], source=SOURCE + "# user edit"
        )
        self.failure(job)
        self.manager.send.assert_not_called()
        self.assertIn("files changed", self.flow.jobs[job["id"]]["error"])

    def test_removed_video_files_during_repair_do_not_leave_it_running(self):
        job = self.start()
        self.failure(job)
        self.flow.read = Mock(side_effect=KeyError("missing"))
        self.finish_turn(job, changed=False)
        self.assertEqual(self.flow.jobs[job["id"]]["status"], "failed")

    def test_generation_can_finish_without_a_browser_and_deduplicates_completion(self):
        turn = {"id": "generated", "status": "completed", "changed": ["script.py"]}
        self.flow.agent_finished(self.thread, turn)
        self.flow.agent_finished(self.thread, turn)
        self.assertEqual(self.submit.call_count, 1)
        self.assertEqual(len(self.flow.jobs), 1)

    def test_two_pdf_workspaces_repair_in_their_own_chats(self):
        first = projects.create_video(self.root / "projects", self.project["id"], "One")
        second = projects.create_video(
            self.root / "projects", self.project["id"], "Two"
        )
        for workspace in (first["workspace"], second["workspace"]):
            projects.update(
                self.root / "projects",
                self.project["id"],
                source=SOURCE,
                workspace=workspace,
            )
            thread = self.manager.create(self.project["id"], workspace=workspace)
            job = self.flow.start(
                self.project["id"], workspace=workspace, thread=thread["id"]
            )
            self.failure(job)
            self.assertEqual(
                self.manager.get(*self.manager.send.call_args.args[:2])["workspace"],
                workspace,
            )
        self.assertEqual(len(self.flow.jobs), 2)
        self.assertEqual(
            projects.read(self.root / "projects", self.project["id"])["source"], SOURCE
        )
