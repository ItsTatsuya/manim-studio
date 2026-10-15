"""Shared fixture and user-level interactions for React workspace checks."""

from contextlib import contextmanager
import json
import os
import re
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from processes import attach_process_tree, release_process_tree, stop_process

SOURCE = "from manim import *\nclass Demo(Scene):\n    def construct(self):\n        self.add(Circle(color=BLUE))\n        self.wait(.2)\n"


@contextmanager
def browser_fixture(*, native_file_picker=False):
    with tempfile.TemporaryDirectory(prefix="manim-react-checks-") as directory:
        folder = Path(directory)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        url = f"http://127.0.0.1:{port}"
        env = {
            **os.environ,
            "MANIM_STUDIO_DATA": str(folder / "data"),
            "FAKE_AGENT_LOG": str(folder / "cli.jsonl"),
        }
        # Native Windows dialogs can retain inherited log handles after the server
        # exits. Keep their diagnostics outside the disposable data fixture.
        log_path = (
            ROOT / ".build-cache" / f"{folder.name}.log"
            if native_file_picker
            else folder / "server.log"
        )
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with log_path.open("w", encoding="utf-8") as log:
            server = subprocess.Popen(
                [
                    sys.executable,
                    str(ROOT / "tests/browser_server.py"),
                    "--port",
                    str(port),
                    *(["--native-file-picker"] if native_file_picker else []),
                ],
                cwd=ROOT,
                env=env,
                stdout=log,
                stderr=log,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
                start_new_session=os.name != "nt",
            )
            try:
                attach_process_tree(server)
                deadline = time.monotonic() + 20
                while True:
                    try:
                        urllib.request.urlopen(url + "/api/health", timeout=1).close()
                        break
                    except OSError:
                        if server.poll() is not None or time.monotonic() > deadline:
                            raise RuntimeError(log_path.read_text(encoding="utf-8"))
                        time.sleep(0.1)
                with sync_playwright() as playwright:
                    browser = playwright.chromium.launch()
                    try:
                        yield browser, url, folder
                    finally:
                        browser.close()
            finally:
                try:
                    stop_process(server)
                    server.wait(timeout=15)
                finally:
                    release_process_tree(server)


def open_page(browser, url, width=1366, height=768):
    context = browser.new_context(viewport={"width": width, "height": height})
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(url)
    expect(page.get_by_role("link", name="Settings", exact=True)).to_be_visible()
    token = page.locator('meta[name="studio-token"]').get_attribute("content")
    context.set_extra_http_headers({"X-Studio-Token": token})
    return context, page, errors


def api(page, url, path, body=None, method=None):
    response = page.request.fetch(
        url + path,
        method=method or ("POST" if body is not None else "GET"),
        data=json.dumps(body) if body is not None else None,
        headers={"Content-Type": "application/json"},
    )
    assert response.ok, (path, response.status, response.text())
    return response.json()


def project(page, url, name="Browser project", source=SOURCE):
    value = api(page, url, "/api/projects", {"name": name, "source": source})
    page.goto(url)
    page.reload()
    expect(page.get_by_role("link", name=name, exact=True)).to_be_visible()
    page.goto(url + "/#/projects/" + value["id"])
    expect(page.locator("main header").first).to_contain_text(name)
    expect(page.get_by_role("tab", name="Preview", exact=True)).to_be_visible()
    page.get_by_role("tab", name="Preview", exact=True).click()
    return value


def select(page, label, option):
    page.get_by_role(
        "combobox", name=re.compile("^" + re.escape(label) + "(?: |$)")
    ).click()
    page.get_by_role("option", name=option, exact=True).click()


def editor(page, text):
    page.get_by_role("tab", name="Script", exact=False).click()
    content = page.locator(".cm-content[contenteditable=true]")
    expect(content).to_be_visible()
    content.click()
    page.keyboard.press("Control+a")
    page.keyboard.insert_text(text)


def wait_api(page, url, path, condition, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = api(page, url, path)
        if condition(value):
            return value
        page.wait_for_timeout(100)
    raise AssertionError((path, value))
