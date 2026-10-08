"""Update notifications, explicit installation and lifecycle/accessibility guards."""

import os
import time
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

URL = os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765")
SOURCE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(.1)\nclass Other(Scene):\n    def construct(self):\n        self.wait(.1)\n"

with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page(viewport={"width": 1366, "height": 768})
    page.clock.install()
    errors, expected_console = [], []
    page.on("pageerror", lambda error: errors.append(str(error)))

    def console(message):
        if message.type != "error":
            return
        if "503" in message.text and message.location.get("url", "").endswith(
            ("/api/preferences", "/api/updates", "/api/updates/install")
        ):
            expected_console.append(message.text)
        else:
            errors.append(message.text)

    page.on("console", console)
    page.on(
        "requestfailed",
        lambda request: (
            errors.append(f"{request.url}: {request.failure}")
            if request.failure != "net::ERR_ABORTED"
            else None
        ),
    )
    saved = {"manim-tour-seen": "2"}
    standalone, fail_save = [True], [False]
    state = {
        "status": "idle",
        "current_version": "1.0.0",
        "latest_version": "1.0.1",
        "edition": "installed",
        "can_download": True,
        "can_install": True,
        "release_url": "https://github.com/ItsTatsuya/manim-studio/releases/tag/v1.0.1",
        "asset_name": "Manim-Studio-Setup.exe",
        "bytes_downloaded": 0,
        "total_bytes": 1000,
    }
    calls, held = [], []
    hold_get = [False]
    held_install, render_calls = [], []
    hold_install = [False]
    fail_snapshot, checking = [False], [False]

    def wait_held():
        deadline = time.monotonic() + 5
        while not held and time.monotonic() < deadline:
            page.wait_for_timeout(10)
        assert len(held) == 1

    def preferences(route):
        if route.request.method == "GET":
            route.fulfill(json=saved)
        elif fail_save[0]:
            route.fulfill(status=503, json={"detail": "Disk write failed"})
        else:
            saved.update(route.request.post_data_json["values"])
            route.fulfill(json=saved)

    def update(route):
        action = route.request.url.split("/api/updates", 1)[1].strip("/")
        calls.append(action or "snapshot")
        if not action and hold_get[0]:
            held.append(route)
            return
        if not action and fail_snapshot[0]:
            route.fulfill(
                status=503, json={"detail": "Local updater temporarily unavailable"}
            )
            return
        if action:
            assert route.request.method == "POST"
            assert route.request.headers.get("x-studio-token")
        if action == "check":
            state.update(status="checking" if checking[0] else "available", error=None)
        elif action == "download":
            state.update(status="downloading", bytes_downloaded=250)
        elif action == "cancel":
            state.update(status="available", bytes_downloaded=0)
        elif action == "install":
            if hold_install[0]:
                held_install.append(route)
                return
            state.update(status="installing")
        route.fulfill(json=state)

    page.route("**/api/preferences", preferences)
    page.route(
        "**/api/health",
        lambda route: route.fulfill(
            json={"manim": True, "latex": True, "standalone": standalone[0]}
        ),
    )
    page.route("**/api/renders", lambda route: route.fulfill(json=[]))
    page.route("**/api/updates", update)
    page.route("**/api/updates/*", update)
    page.route(
        "**/api/render",
        lambda route: (
            render_calls.append(route.request.post_data_json),
            route.fulfill(status=409, json={"detail": "Studio is preparing an update"}),
        ),
    )
    page.goto(URL)
    expect(page.locator("#updates-badge")).to_be_visible()
    assert calls.count("check") == 1
    expect(page.locator("#updates-dialog")).to_be_hidden()
    page.locator("#updates-button").focus()
    page.keyboard.press("Enter")
    expect(page.locator("#updates-close")).to_be_focused()
    expect(page.locator("#updates-version")).to_have_text("Current version: 1.0.0")
    expect(page.locator("#updates-auto")).to_be_checked()
    expect(page.locator("#updates-check")).to_be_enabled()
    page.clock.fast_forward(6 * 60 * 60 * 1000)
    deadline = time.monotonic() + 5
    while calls.count("check") < 2 and time.monotonic() < deadline:
        page.wait_for_timeout(10)
    assert calls.count("check") == 2
    page.locator("#updates-auto").uncheck()
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-check-updates"] == "false"
    page.keyboard.press("Escape")
    expect(page.locator("#updates-button")).to_be_focused()
    count = calls.count("check")
    page.clock.fast_forward(6 * 60 * 60 * 1000)
    page.wait_for_timeout(100)
    assert calls.count("check") == count
    page.reload()
    page.wait_for_function("preferencesReady && window.engineHealth")
    page.evaluate("source=>setSource(source)", SOURCE)
    page.locator("#updates-button").click()
    expect(page.locator("#updates-auto")).not_to_be_checked()
    assert calls.count("check") == count
    print(
        "PASS: packaged automatic notification, explicit opt-out/reload and keyboard focus",
        flush=True,
    )

    page.locator("#updates-download").click()
    expect(page.locator("#updates-progress-text")).to_contain_text("25%")
    expect(page.locator("#updates-check")).to_be_disabled()
    expect(page.locator("#updates-cancel")).to_be_enabled()
    fail_snapshot[0] = True
    expect(page.locator("#updates-status")).to_contain_text("Couldn’t refresh progress")
    expect(page.locator("#updates-progress-text")).to_contain_text("25%")
    expect(page.locator("#updates-check")).to_have_text("Refresh status")
    for delay in (1000, 2000):
        previous = len(expected_console)
        page.clock.fast_forward(delay)
        deadline = time.monotonic() + 5
        while len(expected_console) == previous and time.monotonic() < deadline:
            page.wait_for_timeout(10)
        assert len(expected_console) == previous + 1
    count = calls.count("snapshot")
    page.clock.fast_forward(10000)
    page.wait_for_timeout(100)
    assert calls.count("snapshot") == count, (
        "Transport retries must stop after three failures"
    )
    fail_snapshot[0] = False
    page.locator("#updates-check").click()
    expect(page.locator("#updates-status")).not_to_contain_text(
        "Couldn’t refresh progress"
    )
    expect(page.locator("#updates-progress-text")).to_contain_text("25%")
    page.locator("#updates-cancel").click()
    expect(page.locator("#updates-download")).to_be_enabled()
    page.locator("#updates-download").click()
    expect(page.locator("#updates-progress-text")).to_contain_text("25%")
    state.update(can_cancel=False)
    expect(page.locator("#updates-cancel")).to_be_disabled()
    expect(page.locator("#updates-cancel")).to_have_text("Cancelling…")
    state.update(status="ready", bytes_downloaded=1000)
    expect(page.locator("#updates-install")).to_be_enabled()
    snapshots = calls.count("snapshot")
    page.wait_for_timeout(1600)
    assert calls.count("snapshot") == snapshots, "Ready updates should not idle-poll"
    page.evaluate("activeJob={id:'active-render'};refreshUpdateControls()")
    expect(page.locator("#updates-install")).to_be_disabled()
    expect(page.locator("#updates-status")).to_contain_text("current render")
    page.evaluate("activeJob=null;refreshUpdateControls()")
    fail_save[0] = True
    page.evaluate(
        "document.querySelector('#animation-name').value='Unsaved update draft';document.querySelector('#animation-name').dispatchEvent(new Event('input'))"
    )
    page.locator("#updates-install").click()
    expect(page.locator("#updates-status")).to_contain_text(
        "session could not be saved"
    )
    expect(page.locator("#updates-confirm")).to_be_hidden()
    assert "install" not in calls
    fail_save[0] = False
    page.locator("#updates-install").click()
    expect(page.locator("#updates-confirm")).to_be_visible()
    expect(page.locator("#updates-back")).to_be_focused()
    assert "install" not in calls
    page.locator("#updates-back").click()
    expect(page.locator("#updates-install")).to_be_focused()
    page.keyboard.press("Escape")
    late_formats = []
    page.route("**/api/format", lambda route: late_formats.append(route))
    page.locator("#format-script").click()
    deadline = time.monotonic() + 5
    while not late_formats and time.monotonic() < deadline:
        page.wait_for_timeout(10)
    assert len(late_formats) == 1
    page.evaluate(
        "Object.defineProperty(navigator.clipboard,'readText',{configurable:true,value:()=>new Promise(resolve=>window.resolveLatePaste=resolve)});File.prototype.text=()=>new Promise(resolve=>window.resolveLateImport=resolve);void 0"
    )
    page.locator("#paste-button").click()
    page.locator("#file-input").set_input_files(
        {"name": "late.py", "mimeType": "text/x-python", "buffer": b"# Late import\n"}
    )
    page.wait_for_function(
        "Boolean(window.resolveLatePaste && window.resolveLateImport)"
    )
    page.locator("#updates-button").click()
    page.locator("#updates-install").click()
    hold_install[0] = True
    page.locator("#updates-confirm-install").click()
    expect(page.locator("#updates-status")).to_contain_text("will close and restart")
    deadline = time.monotonic() + 5
    while not held_install and time.monotonic() < deadline:
        page.wait_for_timeout(10)
    assert len(held_install) == 1
    assert calls.count("install") == 1
    assert saved["manim-name"] == "Unsaved update draft"
    page.keyboard.press("Escape")
    expect(page.locator("#render-button")).to_be_disabled()
    expect(page.locator("#render-button span")).to_have_text("Updating Studio…")
    for selector in (
        "#scene-trigger",
        "#scene-select",
        "#resolution-select",
        "#fps-select",
        "#higher-quality",
        "#animation-name",
        "#paste-button",
        "#import-button",
        "#format-script",
        "#examples-button",
        "#theme-button",
        "#updates-auto",
    ):
        expect(page.locator(selector)).to_be_disabled()
    page.keyboard.press("Control+Enter")
    page.evaluate("void startRender()")
    assert not render_calls
    expect(page.locator(".cm-content")).to_have_attribute("contenteditable", "false")
    expect(page.locator(".cm-content")).to_have_attribute("aria-readonly", "true")
    page.locator(".cm-content").focus()
    page.keyboard.type("CHANGED DURING INSTALL")
    page.evaluate(
        "setSource('# replacement','Changed name');replaceSource('# replacement');codeEditor.setSource('# editor replacement')"
    )
    late_formats.pop().fulfill(json={"source": "# delayed format replacement"})
    page.evaluate(
        "resolveLatePaste('# delayed paste');resolveLateImport('# delayed import')"
    )
    expect(page.locator("#format-script")).to_be_disabled()
    assert page.evaluate("codeEditor.getSource()") == SOURCE
    expect(page.locator("#animation-name")).to_have_value("Unsaved update draft")
    assert saved["manim-draft"] == SOURCE
    expect(page.locator("#copy-script")).to_be_enabled()
    expect(page.locator("#confirm-dialog")).to_be_hidden()
    page.evaluate("activeJob={id:'existing-render'};updateRenderButton()")
    expect(page.locator("#render-button span")).to_have_text("Cancel")
    expect(page.locator("#render-button")).to_be_enabled()
    page.evaluate("activeJob=null;updateRenderButton()")
    expect(page.locator("#render-button")).to_be_disabled()
    state.update(status="installing")
    held_install.pop().fulfill(
        status=503, json={"detail": "Install acknowledgment interrupted"}
    )
    hold_install[0] = False
    page.locator("#updates-button").click()
    expect(page.locator("#updates-status")).to_contain_text("Couldn’t refresh progress")
    with page.expect_download() as draft_download:
        page.locator("#updates-save-draft").click()
    assert Path(draft_download.value.path()).read_text(encoding="utf-8") == SOURCE
    expect(page.locator("#render-button")).to_be_disabled()
    expect(page.locator(".cm-content")).to_have_attribute("contenteditable", "false")
    expect(page.locator("#updates-status")).not_to_contain_text(
        "Couldn’t refresh progress", timeout=5000
    )
    assert page.evaluate("window.updatingStudio")
    state.update(
        status="ready", error="Studio could not close. Close and reopen it, then retry."
    )
    expect(page.locator("#render-button")).to_be_enabled()
    assert not page.evaluate("window.updatingStudio")
    expect(page.locator(".cm-content")).to_have_attribute("contenteditable", "true")
    expect(page.locator("#animation-name")).to_be_enabled()
    expect(page.locator("#updates-status")).to_contain_text("Studio could not close")
    expect(page.locator("#updates-install")).to_be_enabled()
    page.locator("#updates-install").click()
    expect(page.locator("#updates-confirm")).to_be_visible()
    page.locator("#updates-back").click()
    assert calls.count("install") == 1
    page.keyboard.press("Escape")
    expect(page.locator("#updates-dialog")).to_be_hidden()
    expect(page.locator("#updates-button")).to_be_focused()
    page.locator(".cm-content").focus()
    expect(page.locator(".cm-content")).to_be_focused()
    page.keyboard.press("Control+End")
    expect(page.locator(".cm-content")).to_be_focused()
    page.keyboard.type("# Editing restored")
    page.wait_for_function("codeEditor.getSource().includes('# Editing restored')")
    print(
        "PASS: progress/cancellation, save guards, preparing blocks rendering, close failure visible/retryable",
        flush=True,
    )

    state.update(status="error", error="GitHub is unavailable. Retry later.")
    page.reload()
    page.wait_for_function("preferencesReady && window.engineHealth")
    page.locator("#updates-button").click()
    expect(page.locator("#updates-status")).to_contain_text("GitHub is unavailable")
    expect(page.locator("#updates-download")).to_be_hidden()
    checking[0] = True
    page.locator("#updates-check").click()
    expect(page.locator("#updates-status")).to_contain_text("Checking GitHub")
    expect(page.locator("#updates-check")).to_be_disabled()
    state.update(status="available")
    checking[0] = False
    expect(page.locator("#updates-status")).to_contain_text("1.0.1 is available")
    page.keyboard.press("Escape")
    hold_get[0] = True
    page.locator("#updates-button").click()
    wait_held()
    count = calls.count("check")
    page.evaluate(
        "document.querySelector('#updates-check').dispatchEvent(new Event('click'))"
    )
    assert calls.count("check") == count
    page.evaluate("window.dispatchEvent(new PageTransitionEvent('pagehide'))")
    held.pop().fulfill(json={**state, "latest_version": "STALE", "status": "current"})
    page.wait_for_timeout(100)
    assert "STALE" not in page.locator("#updates-status").inner_text()
    hold_get[0] = False
    page.evaluate(
        "window.dispatchEvent(new PageTransitionEvent('pageshow',{persisted:true}))"
    )
    expect(page.locator("#updates-status")).to_contain_text("1.0.1 is available")
    page.keyboard.press("Escape")
    print(
        "PASS: retry, single-flight requests and stale snapshots rejected after page exit",
        flush=True,
    )

    standalone[0] = False
    state.update(
        edition="source",
        can_install=False,
        can_download=False,
        release_url="https://untrusted.example/installer.exe",
    )
    saved.pop("manim-check-updates", None)
    page.evaluate(
        "localStorage.removeItem('manim-check-updates');localStorage.removeItem(pendingPreferencesKey)"
    )
    count = calls.count("check")
    page.reload()
    page.wait_for_function("preferencesReady && window.engineHealth")
    page.locator("#updates-button").click()
    expect(page.locator("#updates-support")).to_be_visible()
    expect(page.locator("#updates-download")).to_be_hidden()
    expect(page.locator("#updates-auto")).not_to_be_checked()
    expect(page.locator("#updates-release")).to_have_attribute(
        "href", "https://github.com/ItsTatsuya/manim-studio/releases"
    )
    assert calls.count("check") == count
    print(
        "PASS: source/browser defaults, unsupported installation copy and official release link",
        flush=True,
    )

    page.keyboard.press("Escape")
    standalone[0] = True
    state.update(edition="installed")
    count = calls.count("check")
    page.reload()
    deadline = time.monotonic() + 5
    while calls.count("check") == count and time.monotonic() < deadline:
        page.wait_for_timeout(10)
    assert calls.count("check") == count + 1
    page.locator("#updates-button").click()
    expect(page.locator("#updates-auto")).to_be_checked()
    expect(page.locator("#updates-download")).to_be_hidden()
    expect(page.locator("#updates-support")).to_be_visible()
    print(
        "PASS: packaged browser checks automatically but never offers native installation",
        flush=True,
    )

    held_assets = []
    page.route("**/static/updates.js*", lambda route: held_assets.append(route))
    count = calls.count("check")
    page.reload(wait_until="commit")
    page.wait_for_function("window.studioInitialized")
    assert len(held_assets) == 1
    held_assets.pop().continue_()
    deadline = time.monotonic() + 5
    while calls.count("check") == count and time.monotonic() < deadline:
        page.wait_for_timeout(10)
    assert calls.count("check") == count + 1
    page.unroute("**/static/updates.js*")
    page.locator("#updates-button").click()
    expect(page.locator("#updates-auto")).to_be_checked()
    print(
        "PASS: delayed update asset initializes after engine/preferences are already ready",
        flush=True,
    )

    root = Path(__file__).resolve().parents[1]
    page.add_script_tag(path=str(root / "node_modules/axe-core/axe.min.js"))
    for theme in ("dark", "light"):
        page.evaluate("theme=>applyTheme(theme)", theme)
        for width, height in ((1920, 1080), (1366, 768), (320, 600)):
            page.set_viewport_size({"width": width, "height": height})
            result = page.evaluate(
                "async()=>{await new Promise(requestAnimationFrame);const result=await axe.run(document,{runOnly:{type:'tag',values:['wcag2a','wcag2aa','wcag21a','wcag21aa']}});return result.violations.map(item=>({id:item.id,nodes:item.nodes.map(node=>node.target)}))}"
            )
            assert not result, result
            assert page.evaluate("document.documentElement.scrollWidth") <= width
            rect = page.locator("#updates-dialog").bounding_box()
            assert rect["x"] >= 0 and rect["x"] + rect["width"] <= width
            assert rect["y"] >= 0 and rect["y"] + rect["height"] <= height
            page.keyboard.press("Escape")
            rect = page.locator("#updates-button").bounding_box()
            assert rect["x"] >= 0 and rect["x"] + rect["width"] <= width
            assert rect["width"] >= 32
            expect(page.locator(".topbar h1")).to_be_visible()
            page.locator("#updates-button").click()
            print(
                f"PASS update dialog WCAG AA/layout {theme} {width}x{height}",
                flush=True,
            )
    assert expected_console, "The failed preference-save path was not exercised"
    assert not errors, errors
    browser.close()
