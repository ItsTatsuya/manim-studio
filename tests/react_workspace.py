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
        api(self.page, self.url, "/api/agents/settings", {"auto_render": False})
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

    def test_onboarding_ready_provider_guide_persistence_and_reopen(self):
        p = self.page
        api(p, self.url, "/api/preferences", {"values": {"studio-onboarding": ""}})
        p.reload()
        dialog = p.get_by_role("dialog", name="Welcome to Manim Studio")
        expect(dialog).to_be_visible()
        dialog.get_by_role("button", name="Use Claude Code", exact=True).click()
        dialog.get_by_role("button", name="Continue", exact=True).click()
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
            ("codex", "GPT-Test", "Created an intro scene."),
            ("claude", "Claude Sonnet", "Done."),
        ):
            value = project(p, self.url, f"Document contract {provider}")
            p.get_by_role("button", name=re.compile("^Choose agent and model")).click()
            p.get_by_role("menuitemradio", name=option, exact=True).click()
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
        p.get_by_role("link", name="My renders").click()
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
            ("codex", "GPT-Test", "Created an intro scene."),
            ("claude", "Claude Sonnet", "Done."),
        ):
            value = project(p, self.url, f"CLI {provider}")
            p.get_by_role("button", name=re.compile("^Choose agent and model")).click()
            p.get_by_role("menuitemradio", name=option, exact=True).click()
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

    def test_navigation_highlights_follow_selection_and_reload(self):
        p = self.page
        value = project(p, self.url, "Selected tabs")

        def assert_highlight(selected, other):
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
                expect(
                    p.get_by_role("link", name="Settings", exact=True)
                ).to_have_attribute("aria-current", "page")
                other = nav.get_by_role(
                    "link",
                    name="General" if label == "Providers" else "Providers",
                    exact=True,
                )
                assert_highlight(selected, other)
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
            for label in ("Preview", "Script", "Narration", "Assets", "Output"):
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
            expect(
                tabs.get_by_role("tab", name="Output", exact=True)
            ).to_have_attribute("aria-selected", "true")
            wait_api(
                p,
                self.url,
                "/api/preferences",
                lambda prefs: (
                    json.loads(prefs.get("studio-layout", "{}")).get("tab") == "output"
                ),
            )
            p.reload()
            expect(p.get_by_role("tab", name="Output", exact=True)).to_have_attribute(
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
        self.assertEqual(round(sidebar.bounding_box()["width"]), 264)
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        self.assertEqual(round(sidebar.bounding_box()["width"]), 56)
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
        self.assertEqual(round(sidebar.bounding_box()["width"]), 56)
        p.get_by_role("link", name="Settings", exact=True).click()
        expect(p.get_by_role("heading", name="Providers", exact=True)).to_be_visible()
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        p.get_by_role("link", name="Collapsible sidebar", exact=True).click()
        self.assertIn(value["id"], p.url)
        p.get_by_role("button", name="Expand sidebar", exact=True).focus()
        p.keyboard.press("Enter")
        expect(
            p.get_by_role("button", name="Collapse sidebar", exact=True)
        ).to_be_focused()
        self.assertEqual(round(sidebar.bounding_box()["width"]), 264)
        p.get_by_role("button", name="Collapse sidebar", exact=True).click()
        p.set_viewport_size({"width": 320, "height": 800})
        p.get_by_role("button", name="Projects", exact=True).click()
        expect(
            p.get_by_role("link", name="Collapsible sidebar", exact=True)
        ).to_be_visible()
        self.assertEqual(round(sidebar.bounding_box()["width"]), 264)
        p.get_by_role("link", name="Settings", exact=True).click()
        expect(sidebar).not_to_be_visible()
        p.set_viewport_size({"width": 1366, "height": 768})
        expect(
            p.get_by_role("button", name="Expand sidebar", exact=True)
        ).to_be_visible()
        self.assertEqual(round(sidebar.bounding_box()["width"]), 56)

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
        p.get_by_role("button", name="Download update", exact=True).click()
        expect(p.get_by_role("progressbar", name="Update progress")).to_have_attribute(
            "value", "25"
        )
        p.get_by_role("button", name="Cancel download", exact=True).click()
        p.get_by_role("button", name="Install and restart", exact=True).click()
        expect(p.get_by_role("alertdialog")).to_be_visible()
        p.get_by_role("button", name="Cancel", exact=True).click()
        self.assertNotIn("install", calls)
        p.get_by_role("button", name="Install and restart", exact=True).click()
        p.get_by_role("alertdialog").get_by_role(
            "button", name="Install and restart", exact=True
        ).click()
        expect(p.get_by_text("Studio could not close", exact=True)).to_be_visible()
        expect(
            p.get_by_role("button", name="Install and restart", exact=True)
        ).to_be_enabled()
        self.assertEqual(calls.count("install"), 1)
