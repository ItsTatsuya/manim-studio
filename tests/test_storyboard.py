"""Lesson-plan storyboards: plan-mode turns, review, approval and build."""

import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import agents
import projects
import storyboard
import studio
from test_agents import FakeCliCase
from video_workflow import VideoWorkflows


class StoryboardStoreTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.folder = Path(self.temporary.name)

    def test_approval_follows_the_exact_text(self):
        self.assertEqual(storyboard.read(self.folder)["status"], "none")
        with self.assertRaises(storyboard.StoryboardError):
            storyboard.approve(self.folder)
        saved = storyboard.save(self.folder, "# Lesson\n")
        self.assertEqual((saved["status"], saved["approved"]), ("draft", None))
        approved = storyboard.approve(self.folder)
        self.assertEqual(approved["status"], "approved")
        self.assertTrue(approved["approved"])
        # Approving again is harmless; any edit returns it to draft.
        self.assertEqual(storyboard.approve(self.folder)["status"], "approved")
        self.assertEqual(
            storyboard.save(self.folder, "# Lesson\nMore\n")["status"], "draft"
        )
        storyboard.approve(self.folder)
        (self.folder / storyboard.FILE).write_text("# Edited by hand\n")
        self.assertEqual(storyboard.read(self.folder)["status"], "draft")
        with self.assertRaises(storyboard.StoryboardError):
            storyboard.save(self.folder, "x" * (storyboard.MAX_TEXT + 1))
        # The approval sidecar is hidden, so it is never listed or imported.
        self.assertTrue(storyboard.STATE.startswith("."))

    def test_restore_undoes_video_file_edits_and_keeps_their_times(self):
        (self.folder / "script.py").write_text("original")
        saved = storyboard.guard(self.folder)
        before = (self.folder / "script.py").stat().st_mtime_ns
        (self.folder / "script.py").write_text("changed by a plan turn")
        (self.folder / "narration.txt").write_text("new narration")
        self.assertEqual(
            sorted(storyboard.restore(self.folder, saved)),
            ["narration.txt", "script.py"],
        )
        self.assertEqual((self.folder / "script.py").read_text(), "original")
        self.assertEqual((self.folder / "script.py").stat().st_mtime_ns, before)
        self.assertFalse((self.folder / "narration.txt").exists())
        self.assertEqual(storyboard.restore(self.folder, saved), [])

    def test_instructions_describe_the_lesson_format_compactly(self):
        text = storyboard.instructions(self.folder)
        for part in (
            "storyboard.md",
            "By the end of this video you can",
            "Builds on:",
            "Example <n.nn>.<k>",
            "Key idea",
            "Pause and think",
            "Practice problems",
            "Exam tip",
            "Common mistakes",
            "Scene plan",
            "On screen",
        ):
            self.assertIn(part, text)
        self.assertLess(len(text), 3500)

    def test_imported_packages_keep_the_storyboard_outside_assets(self):
        root = self.folder / "projects"
        files = {
            "Lesson/script.py": "from manim import *\nclass A(Scene):\n    def construct(self):\n        pass\n",
            "Lesson/storyboard.md": "# Plan\n",
        }
        result = projects.import_package(
            root,
            [
                (path, io.BytesIO(text.encode()), len(text.encode()))
                for path, text in files.items()
            ],
        )
        path = root / result["folder"]
        self.assertEqual((path / "storyboard.md").read_text(), "# Plan\n")
        self.assertEqual(projects.list_assets(path / "assets"), [])
        self.assertEqual(storyboard.read(path)["status"], "draft")


class PlanTurnTests(FakeCliCase):
    def test_codex_plan_turn_writes_only_the_storyboard(self):
        pid = self.project["id"]
        folder = self.root / "projects" / "Demo"
        script = (folder / "script.py").read_text(encoding="utf-8")
        thread = self.manager.create(pid, provider="codex")
        turn = self.manager.send(pid, thread["id"], "Plan video 1.01", mode="plan")
        self.assertEqual(turn["mode"], "plan")
        self.wait_idle(thread["id"])
        view = self.manager.view(pid, thread["id"])
        self.assertEqual(view["mode"], "plan")
        self.assertEqual(view["turns"][-1]["changed"], ["storyboard.md"])
        self.assertEqual(view["turns"][-1]["mode"], "plan")
        self.assertEqual((folder / "script.py").read_text(encoding="utf-8"), script)
        self.assertFalse((folder / "narration.txt").read_text(encoding="utf-8"))
        self.assertIn("Video 1.01", (folder / "storyboard.md").read_text("utf-8"))
        first = self.invocations()[0]["prompt"]
        self.assertIn("PLAN MODE", first)
        self.assertNotIn("agent_check.py", first)
        self.assertIn(storyboard.PLAN_REMINDER, first)
        changed = [p for kind, p in self.events if kind == "project.changed"]
        self.assertEqual(changed[-1]["files"], ["storyboard.md"])

        # Switching to Build re-briefs the resumed Codex session once.
        self.manager.send(pid, thread["id"], storyboard.BUILD_PROMPT)
        self.wait_idle(thread["id"])
        second = self.invocations()[1]
        self.assertIn("resume", second["argv"])
        self.assertIn("<studio_instructions>", second["prompt"])
        self.assertIn("agent_check.py", second["prompt"])
        self.assertNotIn(storyboard.PLAN_REMINDER, second["prompt"])
        self.manager.send(pid, thread["id"], "Make it blue")
        self.wait_idle(thread["id"])
        self.assertNotIn("<studio_instructions>", self.invocations()[2]["prompt"])
        self.assertEqual(self.manager.view(pid, thread["id"])["mode"], "build")
        with self.assertRaises(agents.AgentError):
            self.manager.send(pid, thread["id"], "x", mode="draw")

    def test_plan_turn_edits_to_video_files_are_restored(self):
        pid = self.project["id"]
        folder = self.root / "projects" / "Demo"
        thread = self.manager.create(pid, provider="claude")
        self.manager.send(pid, thread["id"], "MISBEHAVE", mode="plan")
        self.wait_idle(thread["id"])
        view = self.manager.view(pid, thread["id"])
        self.assertEqual(
            view["turns"][-1]["changed"], ["assets/dot.svg", "storyboard.md"]
        )
        self.assertEqual((folder / "script.py").read_text(encoding="utf-8"), "")
        notices = [item["text"] for item in view["items"] if item["kind"] == "notice"]
        self.assertTrue(any("restored" in text for text in notices))
        self.assertTrue(
            self.invocations()[0]["prompt"].startswith(storyboard.PLAN_REMINDER)
        )


class PlanWorkflowTests(unittest.TestCase):
    def test_plan_turns_never_queue_a_video(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            project = projects.create(root / "projects", "Plan")
            manager = agents.AgentManager(root, python="python", app_root=root)
            thread = manager.create(project["id"], provider="codex")
            flow = VideoWorkflows(
                manager=lambda: manager,
                read=Mock(),
                prepare=Mock(),
                submit=Mock(),
                render=Mock(),
                cancel_render=Mock(),
                publish=Mock(),
            )
            flow.agent_finished(
                thread,
                {
                    "id": "plan",
                    "status": "completed",
                    "mode": "plan",
                    "changed": ["script.py", "storyboard.md"],
                },
            )
            self.assertEqual(flow.jobs, {})
            flow.read.assert_not_called()


class StoryboardApiTests(FakeCliCase):
    def setUp(self):
        super().setUp()
        for name, value in (
            ("ROOT", self.root),
            ("AGENTS", self.manager),
            ("JOBS", {}),
            ("ACTIVE", None),
        ):
            item = patch.object(studio, name, value)
            item.start()
            self.addCleanup(item.stop)

    def test_review_approve_and_build_in_a_video_workspace(self):
        pid = self.project["id"]
        video = projects.create_video(self.root / "projects", pid, "Lesson 1")
        workspace = video["workspace"]
        folder = projects.workspace_folder(self.root / "projects", pid, workspace)
        self.assertEqual(studio.read_storyboard(pid, workspace)["status"], "none")
        thread = self.manager.create(pid, provider="codex", workspace=workspace)
        studio.start_turn(
            pid, thread["id"], studio.TurnRequest(text="Plan it", mode="plan")
        )
        self.wait_idle(thread["id"])
        self.assertEqual(studio.read_storyboard(pid, workspace)["status"], "draft")
        self.assertFalse((self.root / "projects" / "Demo" / "storyboard.md").exists())

        edited = studio.save_storyboard(
            pid, studio.StoryboardUpdate(text="# Edited plan\n"), workspace
        )
        self.assertEqual(edited["text"], "# Edited plan\n")
        with self.assertRaises(studio.HTTPException) as other_chat:
            studio.approve_storyboard(
                pid,
                studio.StoryboardApproval(thread=thread["id"]),
                None,
            )
        self.assertEqual(other_chat.exception.status_code, 422)

        result = studio.approve_storyboard(
            pid, studio.StoryboardApproval(thread=thread["id"]), workspace
        )
        self.assertEqual(result["storyboard"]["status"], "approved")
        self.assertEqual(result["turn"]["mode"], "build")
        self.wait_idle(thread["id"])
        build = self.invocations()[-1]
        self.assertTrue(build["prompt"].endswith(storyboard.BUILD_PROMPT))
        self.assertNotIn("# Edited plan", build["prompt"])
        self.assertIn("class Intro", (folder / "script.py").read_text("utf-8"))
        self.assertEqual(studio.read_storyboard(pid, workspace)["status"], "approved")

        studio.save_storyboard(
            pid, studio.StoryboardUpdate(text="# Changed after approval\n"), workspace
        )
        self.assertEqual(studio.read_storyboard(pid, workspace)["status"], "draft")

    def test_storyboard_waits_for_the_agent(self):
        pid = self.project["id"]
        thread = self.manager.create(pid, provider="codex")
        self.manager.send(pid, thread["id"], "SLEEP", mode="plan")
        with self.assertRaises(studio.HTTPException) as busy:
            studio.save_storyboard(pid, studio.StoryboardUpdate(text="# Mine\n"))
        self.assertEqual(busy.exception.status_code, 409)
        self.manager.interrupt(pid, thread["id"], wait=True)
        with self.assertRaises(studio.HTTPException) as empty:
            studio.approve_storyboard(pid, studio.StoryboardApproval())
        self.assertEqual(empty.exception.status_code, 422)
        with self.assertRaises(studio.HTTPException) as missing:
            studio.read_storyboard("0" * 32)
        self.assertEqual(missing.exception.status_code, 404)


if __name__ == "__main__":
    unittest.main()
