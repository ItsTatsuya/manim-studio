"""House styles: library, project metadata, generated module and endpoints."""

import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from zipfile import ZipFile

from fastapi import HTTPException

import agents
import projects
import studio
import styles

SCRIPT = """from manim import *
from studio_style import *


class Lesson(Scene):
    def construct(self):
        apply(self)
        heading_text = title("Learning objectives")
        self.play(FadeIn(heading_text), run_time=RUN_TIME)
        key = key_box(body("Key idea"), label="Definition")
        example = example_box(bullets("one", "two"), label="Example")
        self.add(key, example.next_to(key, DOWN), Square(), caption("c"))
        assert self.camera.background_color.to_hex().upper() == BG
        assert Square().color.to_hex().upper() == TEXT
        assert Text("x")[0].get_fill_color().to_hex().upper() == TEXT
        self.wait(WAIT)
"""


class StyleCase(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)


class LibraryTests(StyleCase):
    def test_builtins_and_default(self):
        library = styles.library(self.root)
        self.assertEqual(library["default"], "textbook")
        ids = [style["id"] for style in library["styles"]]
        self.assertEqual(ids, ["textbook", "dark", "high-contrast"])
        self.assertTrue(all(style["builtin"] for style in library["styles"]))
        self.assertEqual(styles.get(self.root, "missing")["id"], "textbook")
        self.assertEqual(styles.get(self.root, None)["id"], "textbook")

    def test_validation(self):
        with self.assertRaises(styles.StyleError):
            styles.clean_style({"colors": {"background": "white"}})
        with self.assertRaises(styles.StyleError):
            styles.clean_style({"sizes": {"title": 500}})
        with self.assertRaises(styles.StyleError):
            styles.clean_style({"fonts": {"heading": 'Bad"Font'}})
        cleaned = styles.clean_style(
            {
                "name": "  Mine\x00 ",
                "colors": {"primary": "#abcdef"},
                "rules": "x" * 9000,
            }
        )
        self.assertEqual(cleaned["name"], "Mine")
        self.assertEqual(cleaned["colors"]["primary"], "#ABCDEF")
        self.assertEqual(cleaned["colors"]["background"], "#FFFFFF")
        self.assertEqual(len(cleaned["rules"]), styles.MAX_RULES)

    def test_user_text_cannot_break_the_generated_module(self):
        import ast

        style = {
            "id": "custom-x",
            **styles.clean_style(
                {"name": 'a\\x"b"""\n\\N{', "rules": '"""\\x', "voice": "\\"}
            ),
        }
        self.assertNotIn("\n", style["name"])
        source = styles.module_source(style, ['Fam"ily', "\\"])
        ast.parse(source)
        self.assertIn("House rules", styles.guide(style))

    def test_create_update_delete(self):
        copy = styles.create(self.root, {}, "dark")
        self.assertEqual(copy["name"], "Dark explainer (copy)")
        self.assertFalse(copy["builtin"])
        self.assertEqual(copy["colors"], styles.get(self.root, "dark")["colors"])
        changed = styles.update(
            self.root, copy["id"], {"name": "Mine", "colors": {"primary": "#112233"}}
        )
        self.assertEqual(changed["colors"]["primary"], "#112233")
        self.assertEqual(changed["colors"]["background"], "#14171C")
        stored = json.loads((self.root / ".studio" / "styles.json").read_text())
        self.assertEqual(stored["styles"][0]["name"], "Mine")
        self.assertEqual(
            styles.set_default(self.root, copy["id"])["default"], copy["id"]
        )
        with self.assertRaises(styles.StyleError):
            styles.update(self.root, "textbook", {"name": "Changed"})
        with self.assertRaises(styles.StyleError):
            styles.delete(self.root, "dark")
        with self.assertRaises(KeyError):
            styles.set_default(self.root, "nope")
        with self.assertRaises(KeyError):
            styles.create(self.root, {}, "nope")
        library = styles.delete(self.root, copy["id"])
        self.assertEqual(library["default"], "textbook")
        self.assertEqual(len(library["styles"]), 3)

    def test_corrupt_library_is_ignored(self):
        path = self.root / ".studio" / "styles.json"
        path.parent.mkdir(parents=True)
        path.write_text(
            json.dumps(
                {
                    "default": "custom-1",
                    "styles": [
                        {"id": "custom-1", "colors": {"text": "bad"}},
                        {"id": "dark", "name": "Shadow"},
                        {"id": "custom-2", "name": "Fine"},
                    ],
                }
            )
        )
        library = styles.library(self.root)
        self.assertEqual(library["default"], "textbook")
        self.assertEqual([s["id"] for s in library["styles"]][3:], ["custom-2"])
        self.assertEqual(styles.get(self.root, "dark")["name"], "Dark explainer")


class ProjectStyleTests(StyleCase):
    def test_project_and_video_style(self):
        home = self.root / "projects"
        project = projects.create(home, "Lesson", style="dark")
        self.assertEqual(projects.read(home, project["id"])["style"], "dark")
        video = projects.create_video(home, project["id"], "Part 1")
        folder = projects.workspace_folder(home, project["id"], video["workspace"])
        self.assertNotIn(
            "style", projects.read(home, project["id"], video["workspace"])
        )
        # A batch video follows its parent project until it picks its own style.
        self.assertEqual(styles.for_folder(self.root, folder)["id"], "dark")
        projects.update(
            home, project["id"], style="high-contrast", workspace=video["workspace"]
        )
        self.assertEqual(styles.for_folder(self.root, folder)["id"], "high-contrast")
        projects.update(home, project["id"], style="")
        self.assertNotIn("style", projects.read(home, project["id"]))
        self.assertEqual(
            styles.for_folder(self.root, projects.folder(home, project["id"]))["id"],
            "textbook",
        )
        with self.assertRaises(projects.ProjectError):
            projects.update(home, project["id"], style="Bad Id")
        # Other updates keep the style.
        projects.update(home, project["id"], style="dark")
        projects.update(home, project["id"], name="Renamed")
        self.assertEqual(projects.read(home, project["id"])["style"], "dark")

    def test_import_keeps_style_id(self):
        buffer = io.BytesIO()
        with ZipFile(buffer, "w") as archive:
            archive.writestr("project.json", json.dumps({"name": "P", "style": "dark"}))
            archive.writestr("script.py", SCRIPT)
        buffer.seek(0)
        imported = projects.import_zip(self.root / "projects", buffer)
        self.assertEqual(imported["style"], "dark")

    def test_module_is_written_only_when_it_changes(self):
        home = self.root / "projects"
        project = projects.create(home, "Lesson")
        folder = projects.folder(home, project["id"])
        styles.write_module(self.root, folder)
        path = folder / styles.MODULE_FILE
        first = path.read_text(encoding="utf-8")
        stamp = path.stat().st_mtime_ns
        self.assertIn('BG = "#FFFFFF"', first)
        styles.write_module(self.root, folder)
        self.assertEqual(path.stat().st_mtime_ns, stamp)
        projects.update(home, project["id"], style="dark")
        guide = styles.prepare(self.root, folder)
        self.assertIn('BG = "#14171C"', path.read_text(encoding="utf-8"))
        self.assertIn('House style "Dark explainer"', guide)
        self.assertIn("from studio_style import *", guide)
        self.assertLess(len(guide), 1600)
        # The module is beside the script, never listed as an asset.
        self.assertEqual(projects.list_assets(folder / projects.ASSETS), [])

    def test_generated_module_runs_in_agent_check(self):
        home = self.root / "projects"
        project = projects.create(home, "Lesson", SCRIPT, style="dark")
        folder = projects.folder(home, project["id"])
        styles.write_module(self.root, folder)
        result = subprocess.run(
            [sys.executable, str(Path(studio.APP_ROOT) / "agent_check.py")],
            cwd=folder,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("OK:", result.stdout)

    @unittest.skipUnless(
        os.name == "nt" and Path(r"C:\Windows\Fonts\arial.ttf").is_file(),
        "Windows font files",
    )
    def test_project_font_family(self):
        home = self.root / "projects"
        project = projects.create(home, "Fonts")
        folder = projects.folder(home, project["id"])
        target = folder / "assets" / "fonts" / "brand.ttf"
        target.parent.mkdir()
        target.write_bytes(Path(r"C:\Windows\Fonts\arial.ttf").read_bytes())
        (folder / "assets" / "notes.ttf").write_bytes(b"not a font")
        self.assertEqual(
            styles.project_fonts(folder),
            [{"family": "Arial", "path": "assets/fonts/brand.ttf"}],
        )
        styles.write_module(self.root, folder)
        self.assertIn(
            '_PROJECT_FAMILIES = ["Arial"]',
            (folder / styles.MODULE_FILE).read_text(encoding="utf-8"),
        )


class StyleEndpointTests(StyleCase):
    def setUp(self):
        super().setUp()
        patcher = patch.object(studio, "ROOT", self.root)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_style_endpoints(self):
        created = studio.create_style(
            studio.StyleCreate(source="textbook", style={"name": "Physics"})
        )
        self.assertEqual(created["name"], "Physics")
        updated = studio.update_style(
            created["id"], studio.StyleUpdate(style={"layout": {"title": "center"}})
        )
        self.assertEqual(updated["layout"]["title"], "center")
        self.assertEqual(
            studio.default_style(studio.StyleDefault(id=created["id"]))["default"],
            created["id"],
        )
        with self.assertRaises(HTTPException) as caught:
            studio.update_style("textbook", studio.StyleUpdate(style={}))
        self.assertEqual(caught.exception.status_code, 422)
        with self.assertRaises(HTTPException) as caught:
            studio.delete_style("custom-missing")
        self.assertEqual(caught.exception.status_code, 404)
        with self.assertRaises(HTTPException) as caught:
            studio.create_style(studio.StyleCreate(style={"colors": {"text": "red"}}))
        self.assertEqual(caught.exception.status_code, 422)
        self.assertEqual(len(studio.list_styles()["styles"]), 4)

    def test_project_style_selection(self):
        project = studio.create_project(studio.ProjectCreate(name="P", style="dark"))
        self.assertEqual(project["style"], "dark")
        with self.assertRaises(HTTPException) as caught:
            studio.create_project(studio.ProjectCreate(name="Q", style="custom-nope"))
        self.assertEqual(caught.exception.status_code, 422)
        with patch.object(studio.AGENTS, "busy_workspace", return_value=False):
            result = studio.update_project(
                project["id"], studio.ProjectUpdate(style="high-contrast")
            )
            self.assertEqual(result["style"], "high-contrast")
            folder = projects.folder(self.root / "projects", project["id"])
            self.assertIn(
                'BG = "#000000"',
                (folder / styles.MODULE_FILE).read_text(encoding="utf-8"),
            )
            with self.assertRaises(HTTPException):
                studio.update_project(project["id"], studio.ProjectUpdate(style="x y"))
        fonts = studio.list_fonts(project["id"])
        self.assertIsInstance(fonts["system"], list)
        self.assertEqual(fonts["project"], [])
        with self.assertRaises(HTTPException) as caught:
            studio.list_fonts("0" * 32)
        self.assertEqual(caught.exception.status_code, 404)


class AgentStyleTests(StyleCase):
    def test_agent_instructions_include_style(self):
        project = projects.create(self.root / "projects", "Lesson", style="dark")
        folder = projects.folder(self.root / "projects", project["id"])
        manager = agents.AgentManager(
            self.root,
            python=sys.executable,
            app_root=Path(studio.APP_ROOT),
            latex=lambda: False,
        )
        self.addCleanup(manager.shutdown)
        turn = SimpleNamespace(
            thread={
                "provider": "codex",
                "session": None,
                "model": "",
                "effort": "",
            },
            project_dir=folder,
        )
        prompt_dir = self.root / "prompt"
        prompt_dir.mkdir()
        _, preamble = manager.build_command(turn, "codex", prompt_dir)
        self.assertIn('House style "Dark explainer"', preamble)
        self.assertTrue((folder / styles.MODULE_FILE).is_file())


if __name__ == "__main__":
    unittest.main()
