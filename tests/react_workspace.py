"""React workspace checks: real API, renders, and deterministic CLI processes."""

import io
import hashlib
import json
import os
from pathlib import Path
import re
import unittest
from zipfile import ZipFile

from playwright.sync_api import expect
from document_fixtures import documents
from browser_support import (
    SOURCE,
    api,
    browser_fixture,
    editor,
    open_page,
    project,
    select,
    wait_api,
)


class WorkspaceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fixture = browser_fixture()
        cls.browser, cls.url, cls.folder = cls.fixture.__enter__()

    @classmethod
    def tearDownClass(cls):
        cls.fixture.__exit__(None, None, None)

    def setUp(self):
        self.context, self.page, self.errors = open_page(self.browser, self.url)
        api(
            self.page,
            self.url,
            "/api/agents/settings",
            {"auto_render": False, "queue_start": "auto", "queue_on_error": "fix"},
        )
        api(
            self.page,
            self.url,
            "/api/preferences",
            {
                "values": {
                    "studio-layout": '{"width":620,"open":true,"tab":"preview"}',
                    "manim-theme": "dark",
                    "studio-sidebar-collapsed": "false",
                }
            },
        )
        self.page.reload()
        expect(
            self.page.get_by_role("link", name="Settings", exact=True)
        ).to_be_visible()

    def tearDown(self):
        self.context.close()
        self.assertFalse(self.errors, self.errors)

    def test_panel_motion_clips_fixed_content_and_respects_reduced_motion(self):
        p = self.page
        project(p, self.url, "Panel animation")
        result = p.evaluate("""async () => {
          const shell = document.querySelector('.studio-panel-shell');
          const inner = shell.firstElementChild;
          const samples = [];
          document.querySelector('[aria-label="Hide panel"]').click();
          const start = performance.now();
          await new Promise(resolve => {
            const frame = () => {
              samples.push([shell.getBoundingClientRect().width, inner.getBoundingClientRect().width]);
              if (performance.now() - start < 320) requestAnimationFrame(frame); else resolve();
            }; requestAnimationFrame(frame);
          });
          return { samples, inert: shell.inert };
        }""")
        widths = [round(width) for width, _ in result["samples"]]
        self.assertGreater(len(set(widths)), 3, widths)
        self.assertEqual(widths[-1], 0)
        self.assertTrue(result["inert"])
        self.assertLess(
            max(w for _, w in result["samples"]) - min(w for _, w in result["samples"]),
            1,
        )
        p.get_by_role("button", name="Show panel", exact=True).click()
        expect(p.get_by_role("tab", name="Preview", exact=True)).to_be_visible()
        p.emulate_media(reduced_motion="reduce")
        self.assertEqual(
            p.locator(".studio-panel-shell").evaluate(
                "el => getComputedStyle(el).transitionDuration"
            ),
            "0s",
        )

    def test_folder_search_selection_and_renders_back(self):
        p = self.page
        first = project(p, self.url, "Folder Alpha")
        project(p, self.url, "Folder Beta")
        search = p.get_by_role("textbox", name="Search projects", exact=True)
        search.fill("Alpha")
        nav = p.get_by_role("navigation", name="Projects", exact=True)
        expect(nav.get_by_role("link", name="Folder Alpha", exact=True)).to_be_visible()
        expect(nav.get_by_role("link", name="Folder Beta", exact=True)).to_have_count(0)
        search.fill("")
        p.get_by_role("button", name="Select folders", exact=True).click()
        p.get_by_role("button", name="Clear", exact=True).click()
        p.get_by_role("textbox", name="Search folders", exact=True).fill("Alpha")
        p.get_by_role("checkbox", name="Folder Alpha", exact=True).check()
        p.keyboard.press("Escape")
        expect(nav.get_by_role("link", name="Folder Alpha", exact=True)).to_be_visible()
        expect(nav.get_by_role("link", name="Folder Beta", exact=True)).to_have_count(0)
        nav.get_by_role("link", name="Folder Alpha", exact=True).click()
        p.get_by_role("link", name="Renders & queue", exact=True).click()
        expect(
            p.get_by_role("heading", name="Renders & queue", exact=True)
        ).to_be_visible()
        p.get_by_role("complementary", name="Studio sidebar").get_by_role(
            "link", name="Back", exact=True
        ).click()
        expect(p.locator("main header").first).to_contain_text("Folder Alpha")
        self.assertIn(first["id"], p.url)

    def test_sidebar_text_and_back_do_not_reflow_during_expansion(self):
        p = self.page
        p.route("**/api/projects", lambda route: route.fulfill(json={"projects": []}))
        p.goto(self.url + "/#/")
        p.reload()
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        empty = sidebar.get_by_text("No projects yet. Create one to start.", exact=True)
        expect(empty).to_be_visible()
        original = empty.bounding_box()
        sample = """async selector => {
          const aside = document.querySelector('aside[aria-label="Studio sidebar"]');
          const frames = [];
          // Pause before painting so CPU load cannot skip the intermediate widths.
          let transition;
          const observer = new MutationObserver(() => {
            transition = aside.getAnimations().find(a => a.transitionProperty === 'width');
            transition?.pause();
          });
          observer.observe(aside, {attributes:true, attributeFilter:['class']});
          aside.querySelector('[aria-label="Expand sidebar"]').click();
          await new Promise(requestAnimationFrame);
          observer.disconnect();
          if (!transition) throw new Error('The sidebar width transition did not start');
          for (const progress of [0, .25, .5, .75, 1]) {
            transition.currentTime = progress * Number(transition.effect.getTiming().duration);
            const box = aside.querySelector(selector).getBoundingClientRect();
            frames.push({sidebar:aside.getBoundingClientRect().width,
              x:box.x,y:box.y,width:box.width,height:box.height});
          }
          transition.finish();
          return frames;
        }"""
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        expect(sidebar).to_have_css("width", "56px")
        frames = p.evaluate(sample, 'nav[aria-label="Projects"] p')
        self.assertGreater(len({round(frame["sidebar"]) for frame in frames}), 3)
        for frame in frames:
            for key in ("x", "y", "width", "height"):
                self.assertAlmostEqual(
                    frame[key], original[key], delta=0.5, msg=str(frame)
                )
        for path in ("/settings/providers", "/renders"):
            p.goto(self.url + "/#" + path)
            back = sidebar.get_by_role("link", name="Back", exact=True)
            expect(back).to_be_visible()
            original = back.bounding_box()
            self.assertGreater(original["width"], 150)
            p.get_by_role("button", name="Collapse sidebar", exact=True).click()
            expect(sidebar).to_have_css("width", "56px")
            frames = p.evaluate(sample, "[data-workspace-back]")
            self.assertGreater(len(frames), 3)
            for frame in frames:
                self.assertAlmostEqual(frame["width"], original["width"], delta=0.5)
                self.assertAlmostEqual(frame["height"], original["height"], delta=0.5)

    def test_generation_repairs_syntax_and_runtime_errors_without_an_open_editor(self):
        p = self.page
        api(p, self.url, "/api/agents/settings", {"auto_render": True})
        api(
            p,
            self.url,
            "/api/preferences",
            {
                "values": {
                    "manim-quality": "preview",
                    "manim-resolution": "854x480",
                    "manim-fps": "15",
                }
            },
        )
        for provider, marker in (
            ("codex", "BROKEN_SYNTAX"),
            ("claude", "BROKEN_RUNTIME"),
        ):
            value = api(p, self.url, "/api/projects", {"name": f"Recover {provider}"})
            thread = api(
                p,
                self.url,
                f"/api/projects/{value['id']}/threads",
                {"provider": provider},
            )
            api(
                p,
                self.url,
                f"/api/projects/{value['id']}/threads/{thread['id']}/turns",
                {"text": f"Create a video {marker}"},
            )
            # No Studio window is present to react to turn completion or errors.
            p.goto("about:blank")
            videos = wait_api(
                p,
                self.url,
                "/api/videos",
                lambda values: any(
                    video["project"] == value["id"]
                    and video["status"] in {"completed", "failed"}
                    for video in values
                ),
                timeout=60,
            )
            video = next(video for video in videos if video["project"] == value["id"])
            self.assertEqual(video["status"], "completed", video)
            self.assertEqual(video["attempt"], 1)
            self.assertEqual(video["thread"], thread["id"])
            detail = api(
                p, self.url, f"/api/projects/{value['id']}/threads/{thread['id']}"
            )
            self.assertEqual(len(detail["turns"]), 2)
            self.assertTrue(
                all(turn["status"] == "completed" for turn in detail["turns"])
            )
            renders = [
                render
                for render in api(p, self.url, "/api/renders")
                if render.get("project") == value["id"]
            ]
            self.assertEqual(len(renders), 1 if marker == "BROKEN_SYNTAX" else 2)
            self.assertTrue(any(render["status"] == "completed" for render in renders))
            invocations = [
                json.loads(line)
                for line in (self.folder / "cli.jsonl").read_text("utf-8").splitlines()
                if value["folder"] in json.loads(line)["cwd"]
            ]
            self.assertEqual(len(invocations), 2)
            self.assertIn("Studio automatic video repair", invocations[-1]["prompt"])
            self.assertIn(
                "Line 2"
                if marker == "BROKEN_SYNTAX"
                else "fixture runtime error to repair",
                invocations[-1]["prompt"],
            )
            p.goto(self.url + f"/#/projects/{value['id']}?thread={thread['id']}")
            expect(
                p.get_by_role("region", name="Project panel").get_by_label(
                    re.compile("^Rendered video:")
                )
            ).to_be_visible()

    def test_manual_queue_holds_reorders_then_fixes_failures_and_finishes(self):
        p = self.page
        api(
            p,
            self.url,
            "/api/agents/settings",
            {"auto_render": True, "queue_start": "manual", "queue_on_error": "fix"},
        )
        api(
            p,
            self.url,
            "/api/preferences",
            {
                "values": {
                    "manim-quality": "preview",
                    "manim-resolution": "854x480",
                    "manim-fps": "15",
                }
            },
        )
        names = {}
        for name, marker in (
            ("Queue first", ""),
            ("Queue broken", "BROKEN_RUNTIME"),
            ("Queue last", ""),
        ):
            value = api(p, self.url, "/api/projects", {"name": name})
            names[value["id"]] = name
            thread = api(
                p,
                self.url,
                f"/api/projects/{value['id']}/threads",
                {"provider": "claude"},
            )
            api(
                p,
                self.url,
                f"/api/projects/{value['id']}/threads/{thread['id']}/turns",
                {"text": f"Create a video {marker}".strip()},
            )
        queue = wait_api(
            p,
            self.url,
            "/api/queue",
            lambda value: len(value["waiting"]) == 3,
            timeout=90,
        )
        # Manual start: nothing renders until the queue is started.
        self.assertTrue(queue["paused"])
        self.assertIsNone(queue["active"])
        # The Queue button beside Render shows the waiting videos in order.
        p.goto(self.url + f"/#/projects/{value['id']}")
        p.get_by_role("button", name="Queue 3, 3 videos in the render queue").click()
        listing = p.get_by_role("list", name="Queued videos")
        expect(listing.get_by_role("listitem")).to_have_count(3)
        expect(listing.get_by_role("listitem").first).to_contain_text("Queue first")
        expect(p.get_by_role("heading", name="Queue on hold")).to_be_visible()
        p.keyboard.press("Escape")
        p.goto(self.url + "/#/renders")
        bar = p.get_by_role("region", name="Render queue")
        expect(bar).to_contain_text("3 videos waiting")
        expect(bar.get_by_role("button", name="Start queue")).to_be_visible()
        rows = {item["id"]: item["name"] for item in api(p, self.url, "/api/renders")}
        last = queue["waiting"][-1]
        p.get_by_role("button", name=f"Move {rows[last]} up").click()
        reordered = wait_api(
            p,
            self.url,
            "/api/queue",
            lambda value: value["waiting"][-2] == last,
        )
        self.assertEqual(set(reordered["waiting"]), set(queue["waiting"]))
        bar.get_by_role("button", name="Start queue").click()
        wait_api(
            p,
            self.url,
            "/api/videos",
            lambda values: (
                len(values) >= 3
                and all(video["status"] == "completed" for video in values)
            ),
            timeout=180,
        )
        settled = wait_api(
            p,
            self.url,
            "/api/queue",
            lambda value: value["active"] is None and not value["waiting"],
        )
        # Back to holding new videos once the started queue is done.
        self.assertTrue(settled["paused"])
        renders = api(p, self.url, "/api/renders")
        for project_id, name in names.items():
            mine = [item for item in renders if item.get("project") == project_id]
            self.assertTrue(
                any(item["status"] == "completed" for item in mine), (name, mine)
            )
            if name == "Queue broken":
                self.assertEqual(len(mine), 2, mine)

    def test_skip_policy_leaves_a_failed_video_and_keeps_going(self):
        p = self.page
        api(
            p,
            self.url,
            "/api/agents/settings",
            {"auto_render": True, "queue_start": "auto", "queue_on_error": "skip"},
        )
        api(
            p,
            self.url,
            "/api/preferences",
            {"values": {"manim-quality": "preview", "manim-resolution": "854x480"}},
        )
        ids = {}
        for name, marker in (("Skip broken", "BROKEN_RUNTIME"), ("Skip fine", "")):
            value = api(p, self.url, "/api/projects", {"name": name})
            ids[name] = value["id"]
            thread = api(
                p,
                self.url,
                f"/api/projects/{value['id']}/threads",
                {"provider": "claude"},
            )
            api(
                p,
                self.url,
                f"/api/projects/{value['id']}/threads/{thread['id']}/turns",
                {"text": f"Create a video {marker}".strip()},
            )
        videos = wait_api(
            p,
            self.url,
            "/api/videos",
            lambda values: (
                len([v for v in values if v["project"] in ids.values()]) == 2
                and all(
                    v["status"] in {"completed", "failed"}
                    for v in values
                    if v["project"] in ids.values()
                )
            ),
            timeout=120,
        )
        by_project = {video["project"]: video for video in videos}
        self.assertEqual(by_project[ids["Skip broken"]]["status"], "failed")
        self.assertEqual(
            by_project[ids["Skip broken"]]["stage"], "Skipped after an error"
        )
        self.assertEqual(by_project[ids["Skip broken"]]["attempt"], 0)
        self.assertEqual(by_project[ids["Skip fine"]]["status"], "completed")

    def test_send_to_llm_repairs_a_script_and_finishes_the_video(self):
        p = self.page
        value = project(
            p,
            self.url,
            "Repair from script",
            source="from manim import *\nclass Broken(Scene)\n    pass\n",
        )
        panel = p.get_by_role("region", name="Project panel")
        expect(
            panel.get_by_role("combobox", name=re.compile("^Animation"))
        ).to_have_count(0)
        panel.get_by_role("tab", name="Script", exact=True).click()
        panel.get_by_role("button", name="Send to LLM", exact=True).click()
        videos = wait_api(
            p,
            self.url,
            "/api/videos",
            lambda values: any(
                video["project"] == value["id"]
                and video["status"] in {"completed", "failed"}
                for video in values
            ),
            timeout=60,
        )
        video = next(video for video in videos if video["project"] == value["id"])
        self.assertEqual(video["status"], "completed", video)
        self.assertEqual(video["attempt"], 1)
        self.assertIn("thread=" + video["thread"], p.url)
        panel.get_by_role("tab", name="Preview", exact=True).click()
        expect(panel.get_by_label(re.compile("^Rendered video:"))).to_be_visible()
        expect(
            panel.get_by_role("combobox", name=re.compile("^Animation"))
        ).to_have_count(0)
        api(
            p,
            self.url,
            f"/api/projects/{value['id']}",
            {"narration": {"enabled": False}},
        )
        p.reload()
        editor(
            p,
            SOURCE
            + "\nclass Outro(Scene):\n    def construct(self):\n        self.wait(.1)\n",
        )
        panel.get_by_role("tab", name="Preview", exact=True).click()
        select(p, "Animation", "Outro")
        panel.get_by_role("button", name="Render", exact=True).click()
        wait_api(
            p,
            self.url,
            "/api/renders",
            lambda values: any(
                render.get("project") == value["id"]
                and render["scene"] == "Outro"
                and render["status"] == "completed"
                for render in values
            ),
            timeout=60,
        )

    def test_repair_from_render_history_preserves_the_pdf_workspace(self):
        p = self.page
        parent = project(p, self.url, "History repair parent")
        pdf = documents()[0]
        response = p.request.post(
            self.url + f"/api/projects/from-pdf?project={parent['id']}",
            data=pdf["buffer"],
            headers={
                "Content-Type": "application/pdf",
                "X-Filename": "History video.pdf",
            },
        )
        self.assertTrue(response.ok, response.text())
        video = response.json()
        workspace = video["workspace"]
        thread = api(
            p,
            self.url,
            f"/api/projects/{parent['id']}/threads",
            {"workspace": workspace, "provider": "claude"},
        )
        broken = "from manim import *\nclass Broken(Scene):\n    def construct(self):\n        raise ValueError('history repair fixture')\n"
        api(
            p,
            self.url,
            f"/api/projects/{parent['id']}?workspace={workspace}",
            {"source": broken, "narration": {"enabled": False}},
        )
        failed = api(
            p,
            self.url,
            "/api/render",
            {
                "source": broken,
                "scene": "Broken",
                "quality": "preview",
                "project": parent["id"],
                "workspace": workspace,
            },
        )
        wait_api(
            p,
            self.url,
            f"/api/renders/{failed['id']}",
            lambda render: render["status"] == "failed",
            timeout=60,
        )
        p.goto(self.url + "/#/renders")
        p.locator("article").filter(
            has=p.get_by_role("button", name="View log", exact=True)
        ).get_by_role("button", name="Send to LLM", exact=True).first.click()
        values = wait_api(
            p,
            self.url,
            "/api/videos",
            lambda values: any(
                value["workspace"] == workspace
                and value["status"] in {"completed", "failed"}
                for value in values
            ),
            timeout=60,
        )
        repaired = next(value for value in values if value["workspace"] == workspace)
        self.assertEqual(repaired["status"], "completed", repaired)
        self.assertEqual(repaired["thread"], thread["id"])
        self.assertEqual(
            api(p, self.url, f"/api/projects/{parent['id']}")["source"], SOURCE
        )

    def test_back_controls_match_in_settings_and_renders(self):
        p = self.page
        value = project(p, self.url, "Consistent return")
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        measure = """async el => { await document.fonts.ready; await Promise.all(document.getAnimations().filter(a=>a.effect?.getTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))); const box=el.getBoundingClientRect(), style=getComputedStyle(el); return {x:box.x,y:box.y,width:box.width,height:box.height,radius:style.borderRadius,background:style.backgroundColor,font:style.fontSize}; }"""
        for collapsed in (False, True):
            if collapsed:
                p.get_by_role("button", name="Collapse sidebar", exact=True).click()
                expect(sidebar).to_have_css("width", "56px")
            baseline = None
            for path in ("/settings/general", "/renders"):
                p.goto(self.url + "/#" + path)
                back = sidebar.get_by_role("link", name="Back", exact=True)
                expect(back).to_be_visible()
                self.assertEqual(
                    p.get_by_role("link", name="Back", exact=True).count(), 1
                )
                p.mouse.move(0, 0)
                bounds = back.evaluate(measure)
                if baseline is None:
                    baseline = bounds
                else:
                    self.assertEqual(bounds, baseline)
                self.assertEqual(bounds["height"], 36)
        for width in (400, 320):
            p.set_viewport_size({"width": width, "height": 800})
            baseline = None
            for path in ("/settings/general", "/renders"):
                p.goto(self.url + "/#" + path)
                back = p.get_by_role("main").get_by_role(
                    "link", name="Back", exact=True
                )
                expect(back).to_be_visible()
                p.mouse.move(0, 0)
                bounds = back.evaluate(measure)
                if baseline is None:
                    baseline = bounds
                else:
                    self.assertEqual(bounds, baseline)
                back.click()
                expect(p.locator("main header").first).to_contain_text(value["name"])

    def test_collapsed_sidebar_groups_actions_and_switches_projects(self):
        p = self.page
        project(p, self.url, "Rail Alpha")
        second = project(p, self.url, "Rail Beta")
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        expect(sidebar).to_have_css("width", "56px")
        expect(
            sidebar.get_by_role("navigation", name="Projects", exact=True)
        ).not_to_be_visible()
        expect(
            sidebar.get_by_role("textbox", name="Search projects", exact=True)
        ).to_have_count(0)
        p.get_by_role("button", name="Import and PDF videos", exact=True).click()
        expect(
            p.get_by_role("menuitem", name="Import project", exact=True)
        ).to_be_visible()
        p.get_by_role("menuitem", name="Videos from PDFs", exact=True).click()
        expect(
            p.get_by_role("dialog", name="Videos from PDFs", exact=True)
        ).to_be_visible()
        p.get_by_role("dialog").get_by_role(
            "button", name="Close", exact=True
        ).last.click()
        p.get_by_role("button", name="Switch project", exact=True).click()
        search = p.get_by_role("textbox", name="Search projects", exact=True)
        expect(search).to_be_focused()
        search.fill("Rail Alpha")
        nav = p.get_by_role("navigation", name="Choose a project", exact=True)
        expect(nav.get_by_role("link", name="Rail Beta", exact=True)).to_have_count(0)
        nav.get_by_role("link", name="Rail Alpha", exact=True).click()
        expect(p.locator("main header").first).to_contain_text("Rail Alpha")
        self.assertNotIn(second["id"], p.url)
        boxes = sidebar.evaluate(
            """aside => [...aside.querySelectorAll('a,button')].filter(el=>el.getClientRects().length && getComputedStyle(el).visibility==='visible').map(el=>{ const b=el.getBoundingClientRect();return {label:el.getAttribute('aria-label'),x:b.x,y:b.y,width:b.width,height:b.height};})"""
        )
        self.assertLessEqual(len(boxes), 8)
        for box in boxes:
            self.assertEqual((box["width"], box["height"]), (36, 36), box)
            self.assertAlmostEqual(
                box["x"] + box["width"] / 2, 28, delta=0.5, msg=str(box)
            )
        for first, following in zip(boxes, boxes[1:]):
            self.assertGreaterEqual(following["y"], first["y"] + first["height"])

    def test_new_project_is_right_of_search_and_folder_row_toggles(self):
        p = self.page
        value = project(p, self.url, "Toggle folder")
        search = p.get_by_role(
            "textbox", name="Search projects", exact=True
        ).bounding_box()
        button = p.get_by_role("button", name="New project", exact=True).bounding_box()
        self.assertGreaterEqual(button["x"], search["x"] + search["width"])
        self.assertEqual(button["y"], search["y"])
        thread = api(
            p,
            self.url,
            f"/api/projects/{value['id']}/threads",
            {"provider": "codex", "title": "Keep this chat"},
        )
        p.goto(self.url + f"/#/projects/{value['id']}?thread={thread['id']}")
        row = p.get_by_role("navigation", name="Projects", exact=True).get_by_role(
            "link", name="Toggle folder", exact=True
        )
        expect(row).to_have_attribute("aria-expanded", "true")
        expect(
            p.get_by_role("button", name="Collapse Toggle folder", exact=True)
        ).to_have_count(0)
        row.click()
        expect(row).to_have_attribute("aria-expanded", "false")
        self.assertIn(thread["id"], p.url)
        row.press("Enter")
        expect(row).to_have_attribute("aria-expanded", "true")
        self.assertIn(thread["id"], p.url)
        row.click()
        wait_api(
            p,
            self.url,
            "/api/preferences",
            lambda prefs: (
                json.loads(prefs.get("manim-project-expanded", "{}")).get(value["id"])
                is False
            ),
        )
        p.reload()
        expect(row).to_have_attribute("aria-expanded", "false")
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        expect(p.get_by_role("complementary", name="Studio sidebar")).to_have_css(
            "width", "56px"
        )
        button = p.get_by_role("button", name="New project", exact=True).bounding_box()
        self.assertAlmostEqual(button["x"] + button["width"] / 2, 28, delta=1)

    def test_usage_popover_reports_real_values_and_provider_errors(self):
        p = self.page
        p.route(
            "**/api/usage?*",
            lambda route: route.fulfill(
                json={
                    "updated": 1792070000,
                    "providers": [
                        {
                            "id": "codex",
                            "name": "Codex",
                            "available": True,
                            "plan": "plus",
                            "error": None,
                            "windows": [
                                {
                                    "id": "weekly",
                                    "label": "Weekly",
                                    "used_percent": 39,
                                    "left_percent": 61,
                                    "resets_at": 1792652259,
                                }
                            ],
                        },
                        {
                            "id": "claude",
                            "name": "Claude Code",
                            "available": False,
                            "plan": None,
                            "windows": [],
                            "error": "Usage isn't available.",
                        },
                    ],
                }
            ),
        )
        p.get_by_role("button", name="Usage limits", exact=True).click()
        expect(p.get_by_text("61% left", exact=True)).to_be_visible()
        expect(
            p.get_by_role("progressbar", name="Codex Weekly used", exact=True)
        ).to_have_attribute("value", "39")
        expect(p.get_by_text("Usage isn't available.", exact=True)).to_be_visible()
        p.get_by_role("button", name="Refresh usage limits", exact=True).click()
        expect(p.get_by_text("61% left", exact=True)).to_be_visible()

    def test_pdf_shared_and_existing_folder_chats_keep_separate_files(self):
        p = self.page
        api(p, self.url, "/api/agents/settings", {"auto_render": True})
        api(
            p,
            self.url,
            "/api/preferences",
            {
                "values": {
                    "manim-quality": "preview",
                    "manim-resolution": "854x480",
                    "manim-fps": "15",
                }
            },
        )
        p.reload()
        p.get_by_role("button", name="Videos from PDFs", exact=True).click()
        dialog = p.get_by_role("dialog", name="Videos from PDFs", exact=True)
        dialog.get_by_role("textbox", name="Project folder name", exact=True).fill(
            "Shared PDF lessons"
        )
        pdf = documents()[0]
        dialog.get_by_label("PDF scripts", exact=True).set_input_files(
            [{**pdf, "name": "Shared One.pdf"}, {**pdf, "name": "Shared Two.pdf"}]
        )
        dialog.get_by_role(
            "button", name="Create and start 2 videos", exact=True
        ).click()
        expect(dialog.get_by_text("Chat started", exact=True)).to_have_count(
            2, timeout=20000
        )
        [parent] = [
            item
            for item in api(p, self.url, "/api/projects")["projects"]
            if item["name"] == "Shared PDF lessons"
        ]
        threads = api(p, self.url, f"/api/projects/{parent['id']}/threads")["threads"]
        self.assertEqual(len(threads), 2)
        self.assertEqual(len({thread["workspace"] for thread in threads}), 2)
        for index, thread in enumerate(threads):
            wait_api(
                p,
                self.url,
                f"/api/projects/{parent['id']}/threads/{thread['id']}",
                lambda value: value["status"] == "completed",
            )
        workspaces = {thread["workspace"] for thread in threads}
        rendered = wait_api(
            p,
            self.url,
            "/api/renders",
            lambda renders: all(
                any(
                    render.get("workspace") == workspace
                    and render["status"] == "completed"
                    for render in renders
                )
                for workspace in workspaces
            ),
            timeout=60,
        )
        self.assertEqual(
            len(
                [render for render in rendered if render.get("workspace") in workspaces]
            ),
            2,
        )
        for index, thread in enumerate(threads):
            path = f"/api/projects/{parent['id']}?workspace={thread['workspace']}"
            api(p, self.url, path, {"source": SOURCE + f"\n# video marker {index}\n"})
        dialog.get_by_role("button", name="Close", exact=True).last.click()
        for index, thread in enumerate(threads):
            p.goto(self.url + f"/#/projects/{parent['id']}?thread={thread['id']}")
            p.get_by_role("tab", name="Script", exact=True).click()
            expect(p.locator(".cm-content")).to_contain_text(f"video marker {index}")
        self.assertEqual(
            api(p, self.url, f"/api/projects/{parent['id']}")["source"], ""
        )
        p.get_by_role("button", name="Videos from PDFs", exact=True).click()
        dialog.get_by_role("button", name="Remove Shared One.pdf", exact=True).click()
        dialog.get_by_role("button", name="Remove Shared Two.pdf", exact=True).click()
        # Start a fresh dialog state by reloading; the saved parent remains available.
        dialog.get_by_role("button", name="Close", exact=True).last.click()
        p.reload()
        p.get_by_role("button", name="Videos from PDFs", exact=True).click()
        dialog.get_by_role("radio", name=re.compile("^Existing folder")).check()
        select(p, "Project folder", "Shared PDF lessons")
        dialog.get_by_label("PDF scripts", exact=True).set_input_files(
            [{**pdf, "name": "Existing Three.pdf"}]
        )
        dialog.get_by_role(
            "button", name="Create and start 1 video", exact=True
        ).click()
        expect(dialog.get_by_text("Chat started", exact=True)).to_be_visible(
            timeout=20000
        )
        self.assertEqual(
            len(api(p, self.url, f"/api/projects/{parent['id']}/threads")["threads"]), 3
        )

    def test_pdf_batch_starts_isolated_chats_and_queues_every_completion(self):
        p = self.page
        api(p, self.url, "/api/agents/settings", {"auto_render": True})
        api(
            p,
            self.url,
            "/api/preferences",
            {
                "values": {
                    "manim-quality": "preview",
                    "manim-resolution": "854x480",
                    "manim-fps": "15",
                }
            },
        )
        p.reload()
        p.get_by_role("button", name="Videos from PDFs", exact=True).click()
        dialog = p.get_by_role("dialog", name="Videos from PDFs", exact=True)
        dialog.get_by_role("radio", name=re.compile("^Separate folders")).check()
        pdf = documents()[0]
        dialog.get_by_label("PDF scripts", exact=True).set_input_files(
            [{**pdf, "name": "Batch One.pdf"}, {**pdf, "name": "Batch Two.pdf"}]
        )
        dialog.get_by_role(
            "button", name="Create and start 2 videos", exact=True
        ).click()
        expect(dialog.get_by_text("Chat started", exact=True)).to_have_count(
            2, timeout=20000
        )
        projects = api(p, self.url, "/api/projects")["projects"]
        batch = [
            item for item in projects if item["name"] in ("Batch One", "Batch Two")
        ]
        self.assertEqual(len(batch), 2)
        for item in batch:
            threads = api(p, self.url, f"/api/projects/{item['id']}/threads")["threads"]
            self.assertEqual(len(threads), 1)
            wait_api(
                p,
                self.url,
                f"/api/projects/{item['id']}/threads/{threads[0]['id']}",
                lambda thread: thread["status"] == "completed",
            )
        ids = {item["id"] for item in batch}
        renders = wait_api(
            p,
            self.url,
            "/api/renders",
            lambda values: all(
                any(render.get("project") == project_id for render in values)
                for project_id in ids
            ),
        )
        self.assertEqual(
            len([render for render in renders if render.get("project") in ids]), 2
        )
        finished = wait_api(
            p,
            self.url,
            "/api/renders",
            lambda values: all(
                any(
                    render.get("project") == project_id
                    and render["status"] in ("completed", "failed")
                    for render in values
                )
                for project_id in ids
            ),
            timeout=60,
        )
        self.assertTrue(
            all(
                render["status"] == "completed"
                for render in finished
                if render.get("project") in ids
            ),
            finished,
        )

    def test_onboarding_ready_provider_guide_persistence_and_reopen(self):
        p = self.page
        api(p, self.url, "/api/preferences", {"values": {"studio-onboarding": ""}})
        p.reload()
        dialog = p.get_by_role("dialog", name="Welcome to Manim Studio")
        expect(dialog).to_be_visible()
        dialog.get_by_role("button", name="Use Claude Code", exact=True).click()
        dialog.get_by_role("button", name="Continue", exact=True).click()
        expect(
            p.get_by_role("dialog", name="Download the narration voice")
        ).to_be_visible()
        if os.environ.get("MANIM_STUDIO_TEST_KOKORO"):
            # The fixture pre-installs the Standard voice: nothing to download.
            expect(
                p.get_by_text("The Standard voice is installed.", exact=False)
            ).to_be_visible()
        else:
            expect(
                p.get_by_role("button", name=re.compile("^Download .* voice$"))
            ).to_be_visible()
        p.get_by_role("button", name="Continue", exact=True).click()
        expect(p.get_by_role("dialog", name="From an idea to a video")).to_be_visible()
        expect(p.get_by_text("Add your references", exact=True)).to_be_visible()
        p.get_by_role("button", name="Continue", exact=True).click()
        expect(
            p.get_by_role("dialog", name="Make your first animation")
        ).to_be_visible()
        p.get_by_role("button", name="Open Studio", exact=True).click()
        expect(p.get_by_role("dialog")).to_have_count(0)
        self.assertEqual(api(p, self.url, "/api/preferences")["studio-onboarding"], "2")
        self.assertEqual(
            api(p, self.url, "/api/agents")["settings"]["default_provider"], "claude"
        )
        api(p, self.url, "/api/agents/settings", {"default_provider": ""})
        p.reload()
        expect(p.get_by_role("dialog")).to_have_count(0)
        p.goto(self.url + "/#/settings/general")
        p.get_by_role("button", name="Open setup", exact=True).click()
        expect(p.get_by_role("dialog", name="Welcome to Manim Studio")).to_be_visible()
        p.get_by_role("button", name="Set up later", exact=True).click()
        expect(p.get_by_role("dialog")).to_have_count(0)

    def test_onboarding_missing_install_login_refresh_and_save_failure(self):
        p = self.page
        snapshot = api(p, self.url, "/api/agents/refresh", {})
        for provider in snapshot["providers"]:
            provider.update(
                installed=False,
                status="error",
                message="Not installed",
                auth={"status": "unauthenticated"},
            )
        calls = []
        p.route("**/api/agents", lambda route: route.fulfill(json=snapshot))
        p.route("**/api/agents/refresh", lambda route: route.fulfill(json=snapshot))
        for action in ("install", "login"):
            p.route(
                "**/api/agents/*/" + action,
                lambda route: (
                    calls.append(route.request.url.rsplit("/", 2)[-2:]),
                    route.fulfill(json={"ok": True}),
                ),
            )
        api(p, self.url, "/api/preferences", {"values": {"studio-onboarding": ""}})
        p.reload()
        dialog = p.get_by_role("dialog", name="Welcome to Manim Studio")
        expect(dialog).to_be_visible()
        codex = dialog.get_by_role("region", name="Codex setup")
        expect(codex.get_by_role("link", name="Node.js LTS")).to_be_visible()
        codex.get_by_role("button", name="Install", exact=True).click()
        expect(p.get_by_text("Installer opened", exact=True)).to_be_visible()
        self.assertEqual(calls, [["codex", "install"]])
        snapshot["providers"][0].update(
            installed=True, status="warning", message="Log in"
        )
        dialog.get_by_role("button", name="Check again", exact=True).click()
        codex.get_by_role("button", name="Log in", exact=True).click()
        self.assertEqual(calls[-1], ["codex", "login"])
        snapshot["providers"][0].update(
            status="ready", message="Ready", auth={"status": "authenticated"}
        )
        dialog.get_by_role("button", name="Check again", exact=True).click()
        expect(
            codex.get_by_role("button", name="Use Codex", exact=True)
        ).to_be_visible()
        p.route(
            "**/api/preferences",
            lambda route: (
                route.fulfill(status=500, json={"detail": "Disk full"})
                if route.request.method == "POST"
                else route.continue_()
            ),
        )
        dialog.get_by_role("button", name="Set up later", exact=True).click()
        expect(p.get_by_text("Couldn't save setup", exact=True)).to_be_visible()
        expect(dialog).to_be_visible()
        p.unroute("**/api/preferences")
        dialog.get_by_role("button", name="Set up later", exact=True).click()
        expect(p.get_by_role("dialog")).to_have_count(0)

    def test_create_edit_empty_save_reload_and_rename(self):
        p = self.page
        p.get_by_role("button", name="New project", exact=True).click()
        p.get_by_role("textbox", name="Project name").fill("UI created")
        p.get_by_role("button", name="Create project", exact=True).click()
        expect(p.get_by_role("heading", name="What should we animate?")).to_be_visible()
        expect(p.locator("main header").first).to_contain_text("UI created")
        pid = p.url.split("/projects/")[-1]
        editor(p, SOURCE)
        wait_api(
            p, self.url, f"/api/projects/{pid}", lambda value: value["source"] == SOURCE
        )
        editor(p, "")
        wait_api(
            p, self.url, f"/api/projects/{pid}", lambda value: value["source"] == ""
        )
        p.reload()
        expect(p.locator(".cm-content")).to_contain_text("# Write a Manim scene")
        p.get_by_role("button", name="UI created actions").click()
        p.get_by_role("menuitem", name="Rename", exact=True).click()
        p.get_by_role("textbox", name="Name", exact=True).fill("Renamed from UI")
        p.get_by_role("button", name="Save", exact=True).click()
        expect(
            p.get_by_role("link", name="Renamed from UI", exact=True)
        ).to_be_visible()

    def test_serial_saves_and_failed_save_retry(self):
        p = self.page
        value = project(p, self.url, "Save regression")
        path = f"/api/projects/{value['id']}"
        held = []

        def hold(route):
            if route.request.method == "POST":
                held.append(route)
            else:
                route.continue_()

        p.route("**" + path, hold)
        first, second = SOURCE + "# first\n", SOURCE + "# second\n"
        editor(p, first)
        p.wait_for_timeout(850)
        self.assertEqual(len(held), 1)
        editor(p, second)
        p.wait_for_timeout(850)
        self.assertEqual(len(held), 1, "New saves must wait for the previous write")
        held[0].continue_()
        p.wait_for_timeout(150)
        self.assertEqual(len(held), 2)
        held[1].fulfill(status=507, json={"detail": "Disk is full"})
        expect(p.get_by_role("button", name="Retry save")).to_be_visible()
        self.assertIn("Disk is full", p.get_by_role("alert").inner_text())
        p.unroute("**" + path, hold)
        p.get_by_role("button", name="Retry save").click()
        wait_api(p, self.url, path, lambda value: value["source"] == second)
        p.reload()
        expect(p.locator(".cm-content")).to_contain_text("# second")

    def test_assets_import_nested_paths_and_delete_confirmation(self):
        p = self.page
        value = project(p, self.url, "Asset checks")
        p.get_by_role("tab", name="Assets", exact=True).click()
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><circle cx="20" cy="20" r="10" fill="blue"/></svg>'
        p.locator('input[type="file"][aria-label="Upload assets"]').set_input_files(
            {"name": "circle.svg", "mimeType": "image/svg+xml", "buffer": svg.encode()}
        )
        expect(p.get_by_role("link", name="circle.svg", exact=True)).to_be_visible()
        p.get_by_role("button", name="Delete circle.svg", exact=True).click()
        p.get_by_role("button", name="Cancel", exact=True).click()
        expect(p.get_by_role("link", name="circle.svg", exact=True)).to_be_visible()
        p.get_by_role("button", name="Delete circle.svg", exact=True).click()
        p.get_by_role("button", name="Delete asset", exact=True).click()
        expect(p.get_by_role("link", name="circle.svg", exact=True)).to_have_count(0)
        archive = io.BytesIO()
        with ZipFile(archive, "w") as package:
            package.writestr("Nested/script.py", SOURCE)
            package.writestr("Nested/narration.txt", "[0:00] A blue circle.")
            package.writestr("Nested/assets/icons/circle.svg", svg)
        p.get_by_role("button", name="Import project", exact=True).click()
        p.locator('input[type="file"][accept=".zip,application/zip"]').set_input_files(
            {
                "name": "Nested.zip",
                "mimeType": "application/zip",
                "buffer": archive.getvalue(),
            }
        )
        expect(p.get_by_role("dialog")).to_have_count(0)
        expect(p.get_by_role("link", name="Nested", exact=True)).to_be_visible()
        p.get_by_role("tab", name="Assets", exact=True).click()
        expect(
            p.get_by_role("link", name="icons/circle.svg", exact=True)
        ).to_be_visible()
        p.get_by_role("tab", name=re.compile("^Narration")).click()
        expect(p.get_by_role("textbox", name="Narration script")).to_have_value(
            "[0:00] A blue circle."
        )
        self.assertNotEqual(value["id"], p.url.split("/projects/")[-1])

    def test_document_attachments_wait_for_upload_and_reach_agent(self):
        p = self.page
        value = project(p, self.url, "Document references")
        base = f"/api/projects/{value['id']}/assets"
        held = []

        def hold(route):
            if route.request.method == "POST":
                held.append(route)
            else:
                route.continue_()

        p.route("**" + base, hold)
        picker = p.get_by_label("Upload reference files")
        self.assertFalse(picker.get_attribute("accept"))
        p.get_by_role("tab", name="Assets", exact=True).click()
        pdf, word = documents()[:2]
        p.get_by_role("button", name="Attach files", exact=True).click()
        expect(p.get_by_role("menuitem", name="Upload images or audio")).to_be_visible()
        with p.expect_file_chooser() as chooser:
            p.get_by_role("menuitem", name="Upload documents", exact=True).click()
        self.assertTrue(chooser.value.is_multiple())
        chooser.value.set_files([pdf, word])
        message = p.get_by_role("textbox", name="Message the agent")
        message.fill("Animate these document references")
        expect(
            p.get_by_role("status").filter(has_text="Uploading 2 files")
        ).to_be_visible()
        expect(p.get_by_role("button", name="Send", exact=True)).to_be_disabled()
        message.press("Enter")
        self.assertEqual(
            api(p, self.url, f"/api/projects/{value['id']}/threads"), {"threads": []}
        )
        held[0].continue_()
        expect(p.get_by_role("button", name="Remove Lesson notes.pdf")).to_be_visible()
        p.wait_for_function(
            "document.querySelector('[data-composer] [role=status]')?.textContent.includes('1 file')"
        )
        self.assertEqual(len(held), 2)
        held[1].continue_()
        expect(
            p.get_by_role("button", name="Remove Résumé (100% #1).docx")
        ).to_be_visible()
        expect(
            p.get_by_role("link", name="Résumé (100% #1).docx", exact=True)
        ).to_be_visible()
        expect(
            p.get_by_role("link", name="Lesson notes.pdf", exact=True)
        ).to_be_visible()
        expect(p.get_by_role("button", name="Send", exact=True)).to_be_enabled()
        p.unroute("**" + base, hold)
        self.assertEqual(
            p.request.get(self.url + base + "/Lesson%20notes.pdf").body(), pdf["buffer"]
        )
        self.assertEqual(
            p.request.get(
                self.url + base + "/R%C3%A9sum%C3%A9%20%28100%25%20%231%29.docx"
            ).body(),
            word["buffer"],
        )
        p.get_by_role("button", name="Send", exact=True).click()
        expect(p.get_by_text("Created an intro scene.", exact=True)).to_be_visible(
            timeout=15000
        )
        records = [
            json.loads(line)
            for line in (self.folder / "cli.jsonl")
            .read_text(encoding="utf-8")
            .splitlines()
        ]
        prompt = next(
            item["prompt"]
            for item in records
            if item["cwd"].endswith("Document references")
        )
        self.assertIn('"assets/Lesson notes.pdf"', prompt)
        self.assertIn('"assets/Résumé (100% #1).docx"', prompt)

    def test_document_upload_failure_retry_and_project_switch(self):
        p = self.page
        first = project(p, self.url, "Pending document")
        base = f"/api/projects/{first['id']}/assets"
        held = []

        def hold(route):
            if route.request.method == "POST":
                held.append(route)
            else:
                route.continue_()

        p.route("**" + base, hold)
        file = {
            "name": "notes.txt",
            "mimeType": "text/plain",
            "buffer": b"Lesson notes",
        }
        picker = p.get_by_label("Upload reference files")
        picker.set_input_files(file)
        expect(
            p.get_by_role("status").filter(has_text="Uploading 1 file")
        ).to_be_visible()
        held[0].fulfill(status=413, json={"detail": "Document is too large"})
        expect(p.get_by_text("Couldn't add notes.txt", exact=True)).to_be_visible()
        expect(p.get_by_role("button", name="Remove notes.txt")).to_have_count(0)
        picker.set_input_files(file)
        expect(
            p.get_by_role("status").filter(has_text="Uploading 1 file")
        ).to_be_visible()
        # Navigate inside the app while the request remains active.
        p.get_by_role("button", name="New project", exact=True).click()
        p.get_by_role("textbox", name="Project name").fill("Other document project")
        p.get_by_role("button", name="Create project", exact=True).click()
        expect(p.locator("main header").first).to_contain_text("Other document project")
        second_id = p.url.split("/projects/")[-1]
        held[1].continue_()
        wait_api(p, self.url, base, lambda result: len(result["assets"]) == 1)
        expect(p.get_by_role("button", name="Remove notes.txt")).to_have_count(0)
        self.assertEqual(
            api(p, self.url, f"/api/projects/{second_id}/assets"), {"assets": []}
        )
        p.unroute("**" + base, hold)

    def test_all_document_bytes_reach_codex_and_claude(self):
        p = self.page
        files = documents()
        for provider, option, reply in (
            ("codex", re.compile("^GPT-Test"), "Created an intro scene."),
            ("claude", re.compile("^Claude Sonnet 5.5"), "Done."),
        ):
            value = project(p, self.url, f"Document contract {provider}")
            p.get_by_role("button", name=re.compile("^Choose agent and model")).click()
            p.get_by_role("menuitemradio", name=option).click()
            p.get_by_label("Upload reference files").set_input_files(files)
            for file in files:
                expect(
                    p.get_by_role("button", name=f"Remove {file['name']}", exact=True)
                ).to_be_visible()
            p.get_by_role("textbox", name="Message the agent").fill(
                "Use every uploaded document"
            )
            p.get_by_role("button", name="Send", exact=True).click()
            expect(p.get_by_text(reply, exact=True)).to_be_visible(timeout=15000)
            records = [
                json.loads(line)
                for line in (self.folder / "cli.jsonl")
                .read_text(encoding="utf-8")
                .splitlines()
            ]
            record = next(
                item
                for item in reversed(records)
                if item["cwd"].endswith(value["folder"])
            )
            self.assertEqual(
                record["references"],
                [
                    {
                        "path": f"assets/{file['name']}",
                        "size": len(file["buffer"]),
                        "sha256": hashlib.sha256(file["buffer"]).hexdigest(),
                    }
                    for file in files
                ],
            )
            p.reload()
            p.get_by_role("tab", name="Assets", exact=True).click()
            for file in files:
                expect(
                    p.get_by_role("link", name=file["name"], exact=True)
                ).to_be_visible()

    def test_document_drop_duplicates_remove_and_cancel(self):
        p = self.page
        value = project(p, self.url, "Dropped documents")
        files = documents()
        transfer = p.evaluate_handle(
            "files => { const data = new DataTransfer(); for (const file of files) data.items.add(new File([new Uint8Array(file.bytes)], file.name, {type: file.mimeType})); return data; }",
            [
                {
                    "name": file["name"],
                    "mimeType": file["mimeType"],
                    "bytes": list(file["buffer"]),
                }
                for file in files
            ],
        )
        p.locator("[data-composer]").dispatch_event("drop", {"dataTransfer": transfer})
        p.get_by_label("Upload reference files").set_input_files(files[0])
        expect(
            p.get_by_role("button", name="Remove Lesson notes (2).pdf", exact=True)
        ).to_be_visible()
        for file in files:
            expect(
                p.get_by_role("button", name=f"Remove {file['name']}", exact=True)
            ).to_be_visible()
        result = wait_api(
            p,
            self.url,
            f"/api/projects/{value['id']}/assets",
            lambda result: len(result["assets"]) == 5,
        )
        self.assertEqual(len(result["assets"]), 5)
        p.get_by_role("button", name="Remove Lesson notes.pdf", exact=True).click()
        p.get_by_role("button", name="Attach files", exact=True).click()
        with p.expect_file_chooser() as chooser:
            p.get_by_role("menuitem", name="Upload documents", exact=True).click()
        chooser.value.set_files([])
        expect(p.locator("[data-composer] [role=status]")).to_have_count(0)
        p.get_by_role("textbox", name="Message the agent").fill(
            "Use the remaining four references"
        )
        p.get_by_role("button", name="Send", exact=True).click()
        expect(p.get_by_text("Created an intro scene.", exact=True)).to_be_visible(
            timeout=15000
        )
        records = [
            json.loads(line)
            for line in (self.folder / "cli.jsonl")
            .read_text(encoding="utf-8")
            .splitlines()
        ]
        refs = next(
            item["references"]
            for item in reversed(records)
            if item["cwd"].endswith(value["folder"])
        )
        self.assertEqual(len(refs), 4)
        self.assertNotIn("assets/Lesson notes.pdf", [ref["path"] for ref in refs])
        self.assertEqual(
            next(
                ref["sha256"]
                for ref in refs
                if ref["path"] == "assets/Lesson notes (2).pdf"
            ),
            hashlib.sha256(files[0]["buffer"]).hexdigest(),
        )

    def test_assets_document_batch_continues_after_one_failure(self):
        p = self.page
        value = project(p, self.url, "Document batch failure")
        base = f"/api/projects/{value['id']}/assets"
        failed = []

        def upload(route):
            if route.request.method == "POST" and not failed:
                failed.append(route.request.headers.get("x-filename"))
                route.fulfill(status=413, json={"detail": "Document is too large"})
            else:
                route.continue_()

        p.route("**" + base, upload)
        p.get_by_role("tab", name="Assets", exact=True).click()
        p.get_by_label("Upload assets").set_input_files(documents())
        for file in documents()[1:]:
            expect(p.get_by_role("link", name=file["name"], exact=True)).to_be_visible()
        expect(
            p.get_by_role("link", name="Lesson notes.pdf", exact=True)
        ).to_have_count(0)
        p.get_by_label("Upload assets").set_input_files(documents()[0])
        expect(
            p.get_by_role("link", name="Lesson notes.pdf", exact=True)
        ).to_be_visible()
        p.unroute("**" + base, upload)

    def test_document_upload_network_retry_and_failed_send_retains_context(self):
        p = self.page
        value = project(p, self.url, "Document network failure")
        base = f"/api/projects/{value['id']}/assets"
        p.route(
            "**" + base,
            lambda route: (
                route.abort("failed")
                if route.request.method == "POST"
                else route.continue_()
            ),
        )
        p.get_by_label("Upload reference files").set_input_files(documents()[0])
        expect(
            p.get_by_text("Couldn't add Lesson notes.pdf", exact=True)
        ).to_be_visible()
        expect(
            p.get_by_role("button", name="Remove Lesson notes.pdf", exact=True)
        ).to_have_count(0)
        p.unroute("**" + base)
        p.get_by_label("Upload reference files").set_input_files(documents()[0])
        expect(
            p.get_by_role("button", name="Remove Lesson notes.pdf", exact=True)
        ).to_be_visible()
        failed = []

        def turns(route):
            if route.request.method == "POST" and not failed:
                failed.append(True)
                route.fulfill(status=503, json={"detail": "Try again"})
            else:
                route.continue_()

        p.route("**/threads/*/turns", turns)
        p.get_by_role("textbox", name="Message the agent").fill(
            "Use this PDF after retry"
        )
        p.get_by_role("button", name="Send", exact=True).click()
        expect(p.get_by_text("Couldn't send the message", exact=True)).to_be_visible()
        expect(
            p.get_by_role("button", name="Remove Lesson notes.pdf", exact=True)
        ).to_be_visible()
        expect(p.get_by_role("textbox", name="Message the agent")).to_have_value(
            "Use this PDF after retry"
        )
        p.get_by_role("button", name="Send", exact=True).click()
        expect(p.get_by_text("Created an intro scene.", exact=True)).to_be_visible(
            timeout=15000
        )
        self.assertEqual(
            len(api(p, self.url, f"/api/projects/{value['id']}/threads")["threads"]), 1
        )

    def test_real_render_custom_fps_playback_history_and_download(self):
        p = self.page
        value = project(p, self.url, "Real custom render")
        select(p, "Output quality", "Custom…")
        for label, text in (
            ("Width", "640"),
            ("Height", "360"),
            ("Frame rate", "23.976"),
        ):
            p.get_by_role("spinbutton", name=label, exact=True).fill(text)
        p.get_by_role("button", name="Use custom output").click()
        expect(
            p.get_by_role("combobox", name=re.compile("^Output quality"))
        ).to_contain_text("23.976")
        p.wait_for_timeout(400)
        p.reload()
        expect(
            p.get_by_role("combobox", name=re.compile("^Output quality"))
        ).to_contain_text("640×360")
        p.get_by_role("button", name="Render", exact=True).last.click()
        result = wait_api(
            p,
            self.url,
            "/api/renders",
            lambda renders: any(
                r.get("project") == value["id"]
                and r["status"] in ("completed", "failed")
                for r in renders
            ),
            timeout=60,
        )
        render = next(r for r in result if r.get("project") == value["id"])
        self.assertEqual(render["status"], "completed", render)
        self.assertEqual(
            (render["width"], render["height"], render["fps"]), (640, 360, 23.976)
        )
        p.wait_for_function("document.querySelector('video')?.readyState >= 2")
        self.assertIsNone(p.locator("video").evaluate("video => video.error"))
        with p.expect_download() as downloaded:
            p.get_by_role("link", name="Save video").click()
        self.assertGreater(Path(downloaded.value.path()).stat().st_size, 1000)
        p.get_by_role("link", name="Renders & queue").click()
        p.get_by_role("button", name="View video", exact=True).first.click()
        expect(p.get_by_role("dialog").locator("video")).to_be_visible()

    def test_format_syntax_error_and_missing_runtime_gate(self):
        p = self.page
        project(p, self.url, "Diagnostics")
        editor(p, "from manim import *\nclass Broken(Scene)\n    pass\n")
        expect(p.locator("#script-status")).to_contain_text("Line 2")
        expect(
            p.get_by_role("button", name="Render", exact=True).first
        ).to_be_disabled()
        p.keyboard.press("Control+Enter")
        editor(
            p,
            "from manim import *\nclass Demo(Scene):\n\tdef construct(self):\n\t\tself.wait(.1)\n",
        )
        p.get_by_role("button", name="Format", exact=True).click()
        expect(p.locator(".cm-content")).to_contain_text(
            "    def construct", use_inner_text=True
        )
        p.route(
            "**/api/health",
            lambda route: route.fulfill(
                json={
                    "manim": "0.21.0",
                    "python": "3.12",
                    "latex": False,
                    "typst": False,
                    "version": "1.2.1",
                }
            ),
        )
        p.reload()
        editor(p, SOURCE.replace("Circle(color=BLUE)", 'MathTex("x")'))
        p.get_by_role("tab", name="Preview", exact=True).click()
        expect(p.get_by_role("tabpanel")).to_contain_text("needs LaTeX")
        expect(
            p.get_by_role("button", name="Render", exact=True).first
        ).to_be_disabled()
        editor(p, SOURCE.replace("Circle(color=BLUE)", 'MathTypst("x")'))
        p.get_by_role("tab", name="Preview", exact=True).click()
        expect(p.get_by_role("tabpanel")).to_contain_text("Typst is unavailable")

    def test_codex_and_claude_generation_resume_and_interrupt(self):
        p = self.page
        for provider, option, reply in (
            ("codex", re.compile("^GPT-Test"), "Created an intro scene."),
            ("claude", re.compile("^Claude Sonnet 5.5"), "Done."),
        ):
            value = project(p, self.url, f"CLI {provider}")
            p.get_by_role("button", name=re.compile("^Choose agent and model")).click()
            p.get_by_role("menuitemradio", name=option).click()
            p.get_by_role("textbox", name="Message the agent").fill(
                "Create a narrated intro scene and an SVG asset"
            )
            p.get_by_role("button", name="Send", exact=True).click()
            expect(p.get_by_text(reply, exact=True)).to_be_visible(timeout=15000)
            tid = p.url.split("thread=")[-1]
            path = f"/api/projects/{value['id']}/threads/{tid}"
            thread = wait_api(
                p, self.url, path, lambda thread: thread["status"] == "completed"
            )
            self.assertEqual(thread["provider"], provider)
            self.assertTrue(thread["locked"])
            doc = api(p, self.url, f"/api/projects/{value['id']}")
            self.assertIn("class Intro", doc["source"])
            self.assertTrue(doc["narration"]["enabled"])
            p.get_by_role("tab", name="Assets", exact=True).click()
            expect(p.get_by_role("link", name="dot.svg", exact=True)).to_be_visible()
            p.get_by_role("textbox", name="Message the agent").fill(
                "Continue the existing scene"
            )
            p.get_by_role("button", name="Send", exact=True).click()
            wait_api(
                p,
                self.url,
                path,
                lambda thread: (
                    len(thread["turns"]) == 2 and thread["status"] == "completed"
                ),
            )
            p.reload()
            expect(p.get_by_role("textbox", name="Message the agent")).to_be_visible()
            p.get_by_role("textbox", name="Message the agent").fill(
                "SLEEP while preparing the scene"
            )
            p.get_by_role("button", name="Send", exact=True).click()
            wait_api(
                p,
                self.url,
                path,
                lambda thread: (
                    len(thread["turns"]) == 3 and thread["status"] == "running"
                ),
                timeout=8,
            )
            expect(p.get_by_role("button", name="Stop", exact=True)).to_be_visible()
            p.get_by_role("tab", name="Script", exact=False).click()
            expect(p.locator(".cm-content")).to_have_attribute(
                "contenteditable", "false"
            )
            p.get_by_role("button", name="Stop", exact=True).click()
            wait_api(
                p, self.url, path, lambda thread: thread["status"] == "interrupted"
            )
            expect(p.locator(".cm-content")).to_have_attribute(
                "contenteditable", "true"
            )
        records = [
            json.loads(line)
            for line in (self.folder / "cli.jsonl")
            .read_text(encoding="utf-8")
            .splitlines()
        ]
        self.assertTrue(any("resume" in item["argv"] for item in records))
        self.assertTrue(any("--resume" in item["argv"] for item in records))

    @unittest.skipUnless(
        os.environ.get("MANIM_STUDIO_TEST_KOKORO"),
        "Set MANIM_STUDIO_TEST_KOKORO for real narrated generation",
    )
    def test_auto_render_with_real_voice_after_generation(self):
        p = self.page
        api(p, self.url, "/api/agents/settings", {"auto_render": True})
        api(
            p,
            self.url,
            "/api/preferences",
            {
                "values": {
                    "manim-quality": "preview",
                    "manim-resolution": "854x480",
                    "manim-fps": "15",
                }
            },
        )
        value = project(p, self.url, "Generated narrated render")
        p.get_by_role("textbox", name="Message the agent").fill(
            "Create a narrated video"
        )
        p.get_by_role("button", name="Send", exact=True).click()
        renders = wait_api(
            p,
            self.url,
            "/api/renders",
            lambda renders: any(
                r.get("project") == value["id"]
                and r["status"] in ("completed", "failed")
                for r in renders
            ),
            timeout=90,
        )
        render = next(r for r in renders if r.get("project") == value["id"])
        self.assertEqual(render["status"], "completed", render)
        self.assertTrue(render["narrated"])
        p.get_by_role("tab", name=re.compile("^Narration")).click()
        expect(p.get_by_role("textbox", name="Narration script")).to_have_value(
            "[0:00] Here is a circle.\n"
        )
        p.get_by_role("button", name="Preview voice", exact=True).click()
        p.wait_for_function(
            "document.querySelector('audio')?.readyState >= 2", timeout=60000
        )
        self.assertIsNone(p.locator("audio").evaluate("audio => audio.error"))

    def test_provider_settings_custom_models_and_theme_persist(self):
        p = self.page
        p.get_by_role("link", name="Settings", exact=True).click()
        p.get_by_role("textbox", name="Codex custom models").fill("custom-model")
        p.get_by_role("button", name="Save provider", exact=True).first.click()
        expect(p.get_by_text("Codex settings saved", exact=True)).to_be_visible()
        p.reload()
        expect(p.get_by_role("textbox", name="Codex custom models")).to_have_value(
            "custom-model"
        )
        p.get_by_role("link", name="General", exact=True).click()
        select(p, "Appearance", "Light")
        p.wait_for_timeout(400)
        p.reload()
        expect(p.locator("html")).not_to_have_class(re.compile("dark"))

    def test_provider_switch_applies_immediately_and_save_tracks_edits(self):
        p = self.page
        p.get_by_role("link", name="Settings", exact=True).click()
        save = p.get_by_role("button", name="Save provider", exact=True).first
        expect(save).to_be_disabled()
        switch = p.get_by_role("switch", name="Enable Codex", exact=True)
        was = switch.get_attribute("aria-checked") == "true"
        try:
            switch.click()
            wait_api(
                p,
                self.url,
                "/api/agents",
                lambda agents: (
                    agents["settings"]["providers"]["codex"]["enabled"] is (not was)
                ),
            )
            # Toggling saves on its own, so there is nothing left to Save.
            expect(save).to_be_disabled()
            p.get_by_role("textbox", name="Codex binary").fill("codex-custom")
            expect(save).to_be_enabled()
            p.get_by_role("button", name="Check again", exact=True).click()
            expect(
                p.get_by_role("button", name="Check again", exact=True)
            ).to_be_enabled()
            expect(p.get_by_role("textbox", name="Codex binary")).to_have_value(
                "codex-custom"
            )
            expect(save).to_be_enabled()
        finally:
            api(
                p,
                self.url,
                "/api/agents/settings",
                {"providers": {"codex": {"enabled": was}}},
            )

    def test_dragging_files_highlights_the_assets_drop_zone(self):
        p = self.page
        project(p, self.url, "Drop zone")
        p.get_by_role("tab", name="Assets", exact=True).click()
        zone = p.get_by_role("tabpanel", name="Assets").locator("> div").first
        shadow = "el => getComputedStyle(el).boxShadow"
        self.assertEqual(zone.evaluate(shadow), "none")
        zone.evaluate(
            """el => {
              const data = new DataTransfer();
              data.items.add(new File(["x"], "a.txt"));
              el.dispatchEvent(new DragEvent("dragenter", {bubbles: true, dataTransfer: data}));
            }"""
        )
        expect(zone).not_to_have_css("box-shadow", "none")
        zone.evaluate(
            """el => {
              const data = new DataTransfer();
              data.items.add(new File(["x"], "a.txt"));
              el.dispatchEvent(new DragEvent("dragleave", {bubbles: true, dataTransfer: data}));
            }"""
        )
        expect(zone).to_have_css("box-shadow", "none")

    def test_navigation_highlights_follow_selection_and_reload(self):
        p = self.page
        value = project(p, self.url, "Selected tabs")

        def assert_highlight(selected, other, underlined=True):
            p.mouse.move(0, 0)
            p.evaluate(
                "async () => { await Promise.all(document.getAnimations().filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))); }"
            )
            styles = [
                item.evaluate(
                    "element => { const style = getComputedStyle(element); return {color: style.color, background: style.backgroundColor, border: style.borderBottomColor, shadow: style.boxShadow}; }"
                )
                for item in (selected, other)
            ]
            self.assertNotEqual(styles[0]["background"], styles[1]["background"])
            self.assertNotEqual(styles[0]["color"], styles[1]["color"])
            if underlined:
                self.assertNotEqual(styles[0]["shadow"], "none")
                self.assertNotEqual(styles[0]["border"], "rgba(0, 0, 0, 0)")

        for theme in ("dark", "light"):
            api(p, self.url, "/api/preferences", {"values": {"manim-theme": theme}})
            p.goto(self.url + "/#/settings/providers")
            p.reload()
            p.wait_for_function(
                "dark => document.documentElement.classList.contains('dark') === dark",
                arg=theme == "dark",
            )
            nav = p.get_by_role("navigation", name="Settings", exact=True)
            for label in ("Providers", "General", "Narration", "About & updates"):
                selected = nav.get_by_role("link", name=label, exact=True)
                selected.click()
                expect(selected).to_have_attribute("aria-current", "page")
                expect(nav.locator('[aria-current="page"]')).to_have_count(1)
                expect(p.get_by_role("link", name="Back", exact=True)).to_be_visible()
                other = nav.get_by_role(
                    "link",
                    name="General" if label == "Providers" else "Providers",
                    exact=True,
                )
                assert_highlight(selected, other, underlined=False)
            p.reload()
            expect(
                nav.get_by_role("link", name="About & updates", exact=True)
            ).to_have_attribute("aria-current", "page")
            p.go_back()
            expect(
                nav.get_by_role("link", name="Narration", exact=True)
            ).to_have_attribute("aria-current", "page")
            p.go_forward()
            expect(
                nav.get_by_role("link", name="About & updates", exact=True)
            ).to_have_attribute("aria-current", "page")
            p.goto(self.url + f"/#/projects/{value['id']}")
            tabs = p.get_by_role("tablist", name="Project panel")
            expect(tabs).to_be_visible()
            p.evaluate("async () => { await document.fonts.ready; }")
            tab_bounds = tabs.evaluate(
                "tabs => [...tabs.querySelectorAll('[role=tab]')].map(tab => { const rect = tab.getBoundingClientRect(); return {x: rect.x, width: rect.width}; })"
            )
            for label in ("Preview", "Script", "Narration", "Assets", "Log"):
                selected = tabs.get_by_role("tab", name=label, exact=True)
                selected.click()
                expect(selected).to_have_attribute("aria-selected", "true")
                expect(tabs.locator('[aria-selected="true"]')).to_have_count(1)
                expect(
                    p.get_by_role("tabpanel", name=label, exact=True)
                ).to_be_visible()
                other = tabs.get_by_role(
                    "tab",
                    name="Script" if label == "Preview" else "Preview",
                    exact=True,
                )
                assert_highlight(selected, other)
                current_bounds = tabs.evaluate(
                    "tabs => [...tabs.querySelectorAll('[role=tab]')].map(tab => { const rect = tab.getBoundingClientRect(); return {x: rect.x, width: rect.width}; })"
                )
                self.assertEqual(tab_bounds, current_bounds)
            selected.focus()
            p.keyboard.press("Home")
            expect(
                tabs.get_by_role("tab", name="Preview", exact=True)
            ).to_have_attribute("aria-selected", "true")
            p.keyboard.press("End")
            expect(tabs.get_by_role("tab", name="Log", exact=True)).to_have_attribute(
                "aria-selected", "true"
            )
            wait_api(
                p,
                self.url,
                "/api/preferences",
                lambda prefs: (
                    json.loads(prefs.get("studio-layout", "{}")).get("tab") == "output"
                ),
            )
            p.reload()
            expect(p.get_by_role("tab", name="Log", exact=True)).to_have_attribute(
                "aria-selected", "true"
            )

    def test_settings_tabs_do_not_shift_between_sections(self):
        p = self.page
        p.get_by_role("link", name="Settings", exact=True).click()
        nav = p.get_by_role("navigation", name="Settings", exact=True)

        def geometry():
            return nav.evaluate(
                "nav => [nav, ...nav.querySelectorAll('a')].map(element => { const rect = element.getBoundingClientRect(); return {x: rect.x, y: rect.y, width: rect.width, height: rect.height}; })"
            )

        for width in (1366, 900, 320):
            p.set_viewport_size({"width": width, "height": 768})
            nav.get_by_role("link", name="Providers", exact=True).click()
            expect(
                p.get_by_role("heading", name="Providers", exact=True)
            ).to_be_visible()
            p.evaluate("async () => { await document.fonts.ready; }")
            baseline = geometry()
            for label in ("General", "Narration", "About & updates", "Providers"):
                selected = nav.get_by_role("link", name=label, exact=True)
                selected.click()
                expect(selected).to_have_attribute("aria-current", "page")
                for before, after in zip(baseline, geometry(), strict=True):
                    for dimension in ("x", "y", "width", "height"):
                        self.assertAlmostEqual(
                            before[dimension],
                            after[dimension],
                            delta=0.1,
                            msg=f"{width}px, {label}, {dimension}",
                        )

    def test_tabs_keyboard_and_small_viewport_navigation(self):
        p = self.page
        project(p, self.url, "Keyboard layout")
        p.get_by_role("tab", name="Preview", exact=True).focus()
        p.keyboard.press("ArrowRight")
        expect(p.get_by_role("tab", name="Script", exact=True)).to_be_focused()
        expect(p.get_by_role("tab", name="Script", exact=True)).to_have_attribute(
            "aria-selected", "true"
        )
        p.set_viewport_size({"width": 320, "height": 800})
        self.assertLessEqual(p.evaluate("document.documentElement.scrollWidth"), 320)
        p.get_by_role("button", name="Hide panel", exact=True).click()
        p.get_by_role("button", name="Attach files", exact=True).click()
        expect(
            p.get_by_role("menuitem", name="Upload documents", exact=True)
        ).to_be_visible()
        self.assertLessEqual(p.evaluate("document.documentElement.scrollWidth"), 320)
        p.keyboard.press("Escape")
        p.get_by_role("button", name="Projects", exact=True).click()
        expect(p.get_by_role("link", name="Settings", exact=True)).to_be_visible()
        p.get_by_role("link", name="Settings", exact=True).click()
        expect(p.get_by_role("heading", name="Providers", exact=True)).to_be_visible()

    def test_sidebar_collapse_persists_and_keeps_navigation(self):
        p = self.page
        value = project(p, self.url, "Collapsible sidebar")
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        expect(sidebar).to_have_css("width", "264px")
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        expect(sidebar).to_have_css("width", "56px")
        wait_api(
            p,
            self.url,
            "/api/preferences",
            lambda prefs: prefs.get("studio-sidebar-collapsed") == "true",
        )
        p.reload()
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        expect(sidebar).to_have_css("width", "56px")
        p.get_by_role("link", name="Settings", exact=True).click()
        expect(p.get_by_role("heading", name="Providers", exact=True)).to_be_visible()
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        p.get_by_role("link", name="Back", exact=True).click()
        expect(
            p.get_by_role("button", name="Switch project", exact=True)
        ).to_be_visible()
        self.assertIn(value["id"], p.url)
        p.get_by_role("button", name="Expand sidebar", exact=True).focus()
        p.keyboard.press("Enter")
        expect(
            p.get_by_role("button", name="Collapse sidebar", exact=True)
        ).to_be_focused()
        expect(sidebar).to_have_css("width", "264px")
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        p.set_viewport_size({"width": 320, "height": 800})
        p.get_by_role("button", name="Projects", exact=True).click()
        expect(
            p.get_by_role("link", name="Collapsible sidebar", exact=True)
        ).to_be_visible()
        expect(sidebar).to_have_css("width", "264px")
        p.get_by_role("link", name="Settings", exact=True).click()
        expect(sidebar).not_to_be_visible()
        p.set_viewport_size({"width": 1366, "height": 768})
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        expect(sidebar).to_have_css("width", "56px")

    def test_sidebar_icons_keep_their_position_when_toggled(self):
        p = self.page
        project(p, self.url, "Stable icons")
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        expect(sidebar).to_have_css("width", "264px")
        script = """aside => Object.fromEntries(
          ['Collapse sidebar', 'Expand sidebar', 'Settings']
            .map(name => {
              const icon = aside.querySelector(`[aria-label="${name}"] svg`);
              if (!icon || !icon.getClientRects().length) return null;
              const box = icon.getBoundingClientRect();
              return [name.replace('Expand', 'Collapse'),
                      [Math.round(box.left + box.width / 2), Math.round(box.top + box.height / 2)]];
            })
            .filter(Boolean))"""
        folder = """aside => {
          const box = aside.querySelector('nav a svg').getBoundingClientRect();
          return [Math.round(box.left + box.width / 2), Math.round(box.top + box.height / 2)];
        }"""
        open_icons = sidebar.evaluate(script)
        open_folder = sidebar.evaluate(folder)
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        expect(sidebar).to_have_css("width", "56px")
        rail_icons = sidebar.evaluate(script)
        self.assertEqual(rail_icons["Collapse sidebar"], open_icons["Collapse sidebar"])
        self.assertEqual(rail_icons["Settings"][0], open_icons["Settings"][0])
        self.assertAlmostEqual(
            p.get_by_role("button", name="Switch project", exact=True).bounding_box()[
                "x"
            ]
            + 18,
            open_folder[0],
            delta=0.5,
        )
        # Every icon is centred in the 56px rail.
        self.assertTrue(all(x == 28 for x, _ in open_icons.values()), open_icons)
        self.assertEqual(open_folder[0], 28)
        # The footer is a row when open and one column in the rail.
        footer = """aside => ['Renders & queue', 'Settings'].map(name => {
          const box = aside.querySelector(`[aria-label="${name}"]`).getBoundingClientRect();
          return [Math.round(box.left), Math.round(box.top)];
        })"""
        rail = sidebar.evaluate(footer)
        self.assertEqual(rail[0][0], rail[1][0])
        self.assertLess(rail[1][1], rail[0][1])
        p.get_by_role("button", name="Expand sidebar", exact=True).click()
        expect(sidebar).to_have_css("width", "264px")
        row = sidebar.evaluate(footer)
        self.assertEqual(row[0][1], row[1][1])
        self.assertLess(row[1][0], row[0][0])

    def test_mobile_sidebar_slides_instead_of_disappearing(self):
        p = self.page
        project(p, self.url, "Drawer motion")
        p.set_viewport_size({"width": 400, "height": 800})
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        wrapper = p.locator("#studio-sidebar")
        expect(sidebar).not_to_be_visible()
        # Closed: parked off-screen by a transform, not removed with display:none.
        self.assertEqual(
            wrapper.evaluate("el => getComputedStyle(el).display"), "block"
        )
        self.assertNotEqual(
            wrapper.evaluate("el => getComputedStyle(el).transform"), "none"
        )
        p.get_by_role("button", name="Projects", exact=True).click()
        expect(sidebar).to_be_visible()
        expect(wrapper).to_have_css("transform", "none")
        p.keyboard.press("Escape")
        expect(sidebar).not_to_be_visible()
        self.assertNotEqual(
            wrapper.evaluate("el => getComputedStyle(el).transform"), "none"
        )

    def test_narration_controls_align_and_save_at_narrow_widths(self):
        p = self.page
        value = project(p, self.url, "Narration control layout")
        p.get_by_role("tab", name=re.compile("^Narration")).click()
        voice = p.get_by_role("combobox", name=re.compile("^Voice (?!quality)"))
        quality = p.get_by_role("combobox", name=re.compile("^Voice quality"))
        speed = p.get_by_role("spinbutton", name="Speaking speed", exact=True)
        preview = p.get_by_role("button", name="Preview voice", exact=True)
        for width in (1366, 900, 320):
            p.set_viewport_size({"width": width, "height": 800})
            expect(preview).to_be_visible()
            boxes = [
                control.bounding_box() for control in (voice, quality, speed, preview)
            ]
            for first, second in ((0, 1), (2, 3)):
                self.assertAlmostEqual(
                    boxes[first]["width"], boxes[second]["width"], delta=1
                )
                self.assertAlmostEqual(boxes[first]["y"], boxes[second]["y"], delta=1)
                self.assertAlmostEqual(
                    boxes[first]["height"], boxes[second]["height"], delta=1
                )
            self.assertAlmostEqual(boxes[0]["x"], boxes[2]["x"], delta=1)
            self.assertAlmostEqual(boxes[1]["x"], boxes[3]["x"], delta=1)
            self.assertLessEqual(
                p.evaluate("document.documentElement.scrollWidth"), width
            )
        quality.click()
        p.get_by_role("option", name=re.compile("^High quality")).click()
        speed.fill("1.15")
        saved = wait_api(
            p,
            self.url,
            f"/api/projects/{value['id']}",
            lambda doc: (
                doc["narration"]["quality"] == "high"
                and doc["narration"]["speed"] == 1.15
            ),
        )
        self.assertEqual(saved["narration"]["speed"], 1.15)

    def test_speed_field_accepts_typed_values_and_repairs_on_blur(self):
        p = self.page
        value = project(p, self.url, "Speed typing")
        p.get_by_role("tab", name=re.compile("^Narration")).click()
        speed = p.get_by_role("spinbutton", name="Speaking speed", exact=True)
        speed.click()
        p.keyboard.press("Control+A")
        # "0" and "0." are out of range on their own; they must stay typed.
        p.keyboard.type("0.75", delay=40)
        expect(speed).to_have_value("0.75")
        saved = wait_api(
            p,
            self.url,
            f"/api/projects/{value['id']}",
            lambda doc: doc["narration"]["speed"] == 0.75,
        )
        self.assertEqual(saved["narration"]["speed"], 0.75)
        p.keyboard.press("Control+A")
        p.keyboard.type("9")
        expect(speed).to_have_value("9")
        p.get_by_role("textbox", name="Narration script", exact=True).click()
        expect(speed).to_have_value("0.75")

    def test_confirm_dialog_keeps_its_content_while_closing(self):
        p = self.page
        project(p, self.url, "Confirm exit")
        p.get_by_role("tab", name="Assets", exact=True).click()
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>'
        p.locator('input[type="file"][aria-label="Upload assets"]').set_input_files(
            {"name": "dot.svg", "mimeType": "image/svg+xml", "buffer": svg.encode()}
        )
        expect(p.get_by_text("1 asset", exact=True)).to_be_visible()
        p.get_by_role("button", name="Delete dot.svg", exact=True).click()
        dialog = p.get_by_role("alertdialog")
        expect(dialog).to_be_visible()
        p.evaluate(
            """() => {
              const node = document.querySelector('[role=alertdialog]');
              window.__seen = [node.textContent];
              new MutationObserver(() => window.__seen.push(node.textContent))
                .observe(node, {subtree: true, childList: true, characterData: true});
            }"""
        )
        dialog.get_by_role("button", name="Cancel", exact=True).click()
        expect(dialog).to_have_count(0)
        seen = p.evaluate("window.__seen")
        for text in seen:
            self.assertIn("Delete dot.svg?", text)
            self.assertIn("Delete asset", text)

    def test_failed_save_recovers_draft_after_reload(self):
        p = self.page
        value = project(p, self.url, "Recovery check")
        path = f"/api/projects/{value['id']}"

        def fail(route):
            if route.request.method == "POST":
                route.fulfill(status=507, json={"detail": "Disk is full"})
            else:
                route.continue_()

        p.route("**" + path, fail)
        editor(p, SOURCE + "# recovered draft\n")
        expect(p.get_by_role("button", name="Retry save")).to_be_visible()
        p.reload()
        expect(p.locator(".cm-content")).to_contain_text("# recovered draft")
        expect(p.get_by_role("button", name="Retry save")).to_be_visible()
        p.unroute("**" + path, fail)
        p.get_by_role("button", name="Retry save").click()
        wait_api(
            p, self.url, path, lambda value: "# recovered draft" in value["source"]
        )

    def test_sidebar_update_notice_downloads_from_the_sidebar(self):
        p = self.page
        state = {
            "status": "available",
            "progress": 0,
            "bytes_downloaded": 0,
            "total_bytes": 100,
            "error": None,
            "current_version": "1.2.1",
            "edition": "portable",
            "latest_version": "1.2.2",
            "release_url": "https://github.com/ItsTatsuya/manim-studio/releases",
            "release_summary": "Calmer sidebar and faster renders.",
            "release_notes": [f"fix(web): change number {n}" for n in range(1, 12)],
            "last_checked": 1,
            "can_download": True,
            "can_install": False,
            "can_cancel": False,
        }
        calls = []

        def updates(route):
            kind = route.request.url.split("/api/updates")[-1].strip("/")
            if route.request.method == "POST":
                calls.append(kind)
                if kind == "download":
                    state.update(
                        status="downloading",
                        progress=40,
                        can_download=False,
                        can_cancel=True,
                    )
                if kind == "cancel":
                    state.update(status="available", progress=0, can_cancel=False)
                    state.update(can_download=True)
            route.fulfill(json=state)

        p.route("**/api/updates**", updates)
        p.reload()
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        # The update button carries a dot and opens a popover with the notes.
        sidebar.get_by_role("button", name="Update 1.2.2 available").click()
        popup = p.get_by_role("dialog")
        expect(
            popup.get_by_role("heading", name="Update ready to download")
        ).to_be_visible()
        expect(popup.get_by_text("1.2.1 → 1.2.2")).to_be_visible()
        expect(popup.get_by_text("Calmer sidebar and faster renders.")).to_be_visible()
        expect(popup.get_by_role("heading", name="What's changed")).to_be_visible()
        # Eight changes are listed; the rest are a link to the release.
        expect(popup.get_by_role("listitem")).to_have_count(8)
        expect(
            popup.get_by_text("fix(web): change number 1", exact=True)
        ).to_be_visible()
        expect(
            popup.get_by_role("link", name="3 more changes on GitHub")
        ).to_have_attribute("href", state["release_url"])
        popup.get_by_role("button", name="Download update").click()
        expect(
            popup.get_by_role("progressbar", name="Update progress")
        ).to_have_attribute("value", "40")
        popup.get_by_role("button", name="Cancel download").click()
        expect(popup.get_by_role("button", name="Download update")).to_be_visible()
        self.assertEqual(calls, ["download", "cancel"])

    def test_up_to_date_sidebar_offers_a_manual_check_only(self):
        p = self.page
        state = {
            "status": "current",
            "progress": 0,
            "bytes_downloaded": 0,
            "total_bytes": 0,
            "error": None,
            "current_version": "1.0.0",
            "edition": "portable",
            "latest_version": None,
            "release_url": "https://github.com/ItsTatsuya/manim-studio/releases",
            "last_checked": 1,
            "can_download": False,
            "can_install": False,
            "can_cancel": False,
        }
        p.route("**/api/updates**", lambda route: route.fulfill(json=state))
        p.reload()
        sidebar = p.get_by_role("complementary", name="Studio sidebar")
        expect(sidebar.get_by_role("button", name="Download update")).to_have_count(0)
        expect(
            sidebar.get_by_role("button", name=re.compile("Check for updates"))
        ).to_be_visible()

    def test_chat_shows_model_effort_and_forks_from_a_turn(self):
        p = self.page
        value = project(p, self.url, "Fork check")
        p.get_by_role("button", name=re.compile("^Choose agent and model")).click()
        p.get_by_role("menuitemradio", name=re.compile("^Claude Sonnet 5.5")).click()
        p.get_by_role("textbox", name="Message the agent").fill("Create an intro")
        p.get_by_role("button", name="Send", exact=True).click()
        expect(p.get_by_text("Done.", exact=True)).to_be_visible(timeout=15000)
        log = p.get_by_role("log", name="Conversation")
        expect(log.get_by_text(re.compile("Claude Sonnet 5.5 ·"))).to_be_visible()
        before = p.url
        log.get_by_role("button", name="Fork from here").click()
        expect(log.get_by_text(re.compile("^Forked from"))).to_be_visible()
        self.assertNotEqual(p.url, before)
        threads = api(p, self.url, f"/api/projects/{value['id']}/threads")["threads"]
        self.assertEqual(len(threads), 2)

    def test_plan_mode_storyboard_review_edit_and_approve(self):
        p = self.page
        value = project(p, self.url, "Lesson plan")
        p.get_by_role("combobox", name=re.compile("^Chat mode")).click()
        p.get_by_role("option", name=re.compile("^Plan")).click()
        p.get_by_role("textbox", name="Message the agent").fill(
            "Plan video 1.01 on solving two linear equations"
        )
        p.get_by_role("button", name="Send", exact=True).click()
        expect(p).to_have_url(re.compile("thread="))
        tid = p.url.split("thread=")[-1]
        path = f"/api/projects/{value['id']}/threads/{tid}"
        thread = wait_api(
            p, self.url, path, lambda thread: thread["status"] == "completed"
        )
        self.assertEqual(thread["turns"][-1]["mode"], "plan")
        self.assertEqual(thread["turns"][-1]["changed"], ["storyboard.md"])
        log = p.get_by_role("log", name="Conversation")
        expect(log.get_by_text("Plan", exact=True)).to_be_visible()
        log.get_by_role("button", name="storyboard.md").click()
        panel = p.get_by_role("tabpanel", name="Storyboard")
        expect(
            panel.get_by_role("heading", name=re.compile("Two Equations, Two Lines"))
        ).to_be_visible()
        expect(panel.get_by_text("Draft", exact=True)).to_be_visible()
        # No render was started for the plan.
        self.assertFalse(
            [
                r
                for r in api(p, self.url, "/api/renders")
                if r.get("project") == value["id"]
            ]
        )

        panel.get_by_role("button", name="Edit").click()
        board = panel.get_by_role("textbox", name="Storyboard markdown")
        board.fill(board.input_value() + "\n### Extra section\nAdded by hand.\n")
        panel.get_by_role("button", name="Save", exact=True).click()
        expect(panel.get_by_role("heading", name="Extra section")).to_be_visible()
        stored = api(p, self.url, f"/api/projects/{value['id']}/storyboard")
        self.assertIn("Added by hand.", stored["text"])

        panel.get_by_role("textbox", name="Storyboard feedback").fill(
            "Add a pause and think question"
        )
        panel.get_by_role("button", name="Revise with agent").click()
        wait_api(
            p,
            self.url,
            path,
            lambda thread: (
                len(thread["turns"]) == 2 and thread["status"] == "completed"
            ),
        )
        expect(panel.get_by_text(re.compile("Add a pause and think"))).to_be_visible()

        panel.get_by_role("button", name="Approve & generate video").click()
        thread = wait_api(
            p,
            self.url,
            path,
            lambda thread: (
                len(thread["turns"]) == 3 and thread["status"] == "completed"
            ),
        )
        self.assertEqual(thread["turns"][-1]["mode"], "build")
        expect(panel.get_by_text("Approved", exact=True)).to_be_visible()
        doc = api(p, self.url, f"/api/projects/{value['id']}")
        self.assertIn("class Intro", doc["source"])
        expect(
            p.get_by_role("combobox", name=re.compile("^Chat mode"))
        ).to_contain_text("Build")

    def test_update_progress_cancel_confirmation_and_failed_restart(self):
        p = self.page
        state = {
            "status": "available",
            "progress": 0,
            "bytes_downloaded": 0,
            "total_bytes": 100,
            "error": None,
            "current_version": "1.2.1",
            "edition": "portable",
            "latest_version": "1.2.2",
            "release_url": "https://github.com/ItsTatsuya/manim-studio/releases",
            "can_download": True,
            "can_install": False,
            "can_cancel": False,
        }
        calls = []

        def updates(route):
            kind = route.request.url.split("/api/updates")[-1].strip("/")
            if route.request.method == "POST":
                self.assertTrue(route.request.headers.get("x-studio-token"))
                calls.append(kind)
                if kind == "download":
                    state.update(
                        status="downloading",
                        progress=25,
                        bytes_downloaded=25,
                        can_download=False,
                        can_cancel=True,
                    )
                if kind == "cancel":
                    state.update(
                        status="ready", progress=100, can_cancel=False, can_install=True
                    )
                if kind == "install":
                    state.update(status="error", error="Studio could not close")
            route.fulfill(json=state)

        p.route("**/api/updates**", updates)
        p.reload()
        p.get_by_role("link", name="Settings", exact=True).click()
        p.get_by_role("link", name="About & updates", exact=True).click()
        main = p.get_by_role("main")
        main.get_by_role("button", name="Download update", exact=True).click()
        expect(
            main.get_by_role("progressbar", name="Update progress")
        ).to_have_attribute("value", "25")
        main.get_by_role("button", name="Cancel download", exact=True).click()
        main.get_by_role("button", name="Install and restart", exact=True).click()
        expect(p.get_by_role("alertdialog")).to_be_visible()
        p.get_by_role("button", name="Cancel", exact=True).click()
        self.assertNotIn("install", calls)
        main.get_by_role("button", name="Install and restart", exact=True).click()
        p.get_by_role("alertdialog").get_by_role(
            "button", name="Install and restart", exact=True
        ).click()
        expect(main.get_by_text("Studio could not close", exact=True)).to_be_visible()
        expect(
            main.get_by_role("button", name="Install and restart", exact=True)
        ).to_be_enabled()
        self.assertEqual(calls.count("install"), 1)
