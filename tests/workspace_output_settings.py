"""Independent output choices, custom dialogs and persisted render settings."""

import os
import time
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

URL = os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765")
SOURCE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(.1)\n"
OTHER = SOURCE.replace("MainScene", "OtherScene")

with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    context = browser.new_context(viewport={"width": 1366, "height": 768})
    page = context.new_page()
    errors = []
    validation_console = []
    page.on("pageerror", lambda error: errors.append(str(error)))

    def console(message):
        if message.type != "error":
            return
        if "422" in message.text and message.location.get("url", "").endswith(
            "/api/render"
        ):
            validation_console.append(message.text)
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
    saved = {"manim-draft": SOURCE, "manim-tour-seen": "2", "manim-quality": "final"}
    pending_preferences = []
    delay_preferences = [False]

    def wait_routes(routes, count=1):
        deadline = time.monotonic() + 5
        while len(routes) < count and time.monotonic() < deadline:
            page.wait_for_timeout(10)
        assert len(routes) == count

    def preferences(route):
        if route.request.method == "GET":
            if delay_preferences[0]:
                pending_preferences.append(route)
            else:
                route.fulfill(json=saved)
        else:
            saved.update(route.request.post_data_json["values"])
            route.fulfill(json=saved)

    page.route("**/api/preferences", preferences)
    page.route(
        "**/api/health",
        lambda route: route.fulfill(json={"manim": True, "latex": True}),
    )
    page.route("**/api/renders", lambda route: route.fulfill(json=[]))
    page.goto(URL)
    page.wait_for_function(
        "window.codeEditor && analysis?.scenes.length && preferencesReady"
    )
    expect(page.locator("#resolution-select")).to_have_value("1920x1080")
    expect(page.locator("#fps-select")).to_have_value("60")
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-resolution"] == "1920x1080"
    assert saved["manim-fps"] == "60"
    for quality, dimensions, fps in (
        ("preview", "854x480", "15"),
        ("standard", "1280x720", "30"),
        ("final", "1920x1080", "60"),
    ):
        saved["manim-quality"] = quality
        saved.pop("manim-resolution", None)
        saved.pop("manim-fps", None)
        page.evaluate(
            "localStorage.removeItem('manim-resolution');localStorage.removeItem('manim-fps');localStorage.removeItem(pendingPreferencesKey)"
        )
        page.reload()
        page.wait_for_function("preferencesReady && analysis?.scenes.length")
        expect(page.locator("#resolution-select")).to_have_value(dimensions)
        expect(page.locator("#fps-select")).to_have_value(fps)
        page.evaluate("async()=>{await persistPreferences()}")
    print(
        "PASS: legacy quality preferences migrate to independent settings", flush=True
    )

    page.locator("#resolution-select").focus()
    page.keyboard.press("End")
    expect(page.locator("#resolution-dialog")).to_be_visible()
    expect(page.locator("#custom-width")).to_be_focused()
    page.locator("#custom-width").fill("1501")
    page.locator("#custom-height").fill("900")
    page.locator("#resolution-form button[type=submit]").click()
    expect(page.locator("#resolution-dialog")).to_be_visible()
    assert not page.locator("#custom-width").evaluate("element=>element.validity.valid")
    page.keyboard.press("Escape")
    expect(page.locator("#resolution-select")).to_be_focused()
    expect(page.locator("#resolution-select")).to_have_value("1920x1080")
    page.locator("#resolution-select").select_option("custom")
    page.locator("#custom-width").fill("1500")
    page.locator("#custom-height").fill("900")
    page.locator("#resolution-form button[type=submit]").click()
    expect(page.locator("#resolution-select")).to_have_value("1500x900")
    expect(page.locator("#resolution-select")).to_be_focused()
    page.locator("#fps-select").select_option("custom")
    expect(page.locator("#custom-fps")).to_be_focused()
    page.locator("#custom-fps").fill("0")
    page.locator("#fps-form button[type=submit]").click()
    expect(page.locator("#fps-dialog")).to_be_visible()
    assert not page.locator("#custom-fps").evaluate("element=>element.validity.valid")
    page.locator("#custom-fps").fill("17.5")
    page.locator("#fps-form button[type=submit]").click()
    expect(page.locator("#fps-select")).to_have_value("17.5")
    expect(page.locator("#fps-select")).to_be_focused()
    page.locator("#fps-select").select_option("custom")
    page.locator("#custom-fps").fill("19.25")
    page.locator("#fps-form button[type=button]").click()
    expect(page.locator("#fps-select")).to_have_value("17.5")
    page.evaluate("async()=>{await persistPreferences()}")
    page.reload()
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    expect(page.locator("#resolution-select")).to_have_value("1500x900")
    expect(page.locator("#fps-select")).to_have_value("17.5")
    assert page.locator('#resolution-select option[value="1500x900"]').count() == 1
    assert page.locator('#fps-select option[value="17.5"]').count() == 1
    print(
        "PASS: keyboard custom resolution, even validation, fractional FPS, cancel/reload",
        flush=True,
    )

    delay_preferences[0] = True
    page.reload()
    page.wait_for_function("window.codeEditor")
    page.locator("#resolution-select").select_option("1280x720")
    page.locator("#fps-select").select_option("29.97")
    wait_routes(pending_preferences)
    delay_preferences[0] = False
    pending_preferences.pop().fulfill(
        json={**saved, "manim-resolution": "3840x2160", "manim-fps": "120"}
    )
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    expect(page.locator("#resolution-select")).to_have_value("1280x720")
    expect(page.locator("#fps-select")).to_have_value("29.97")
    print(
        "PASS: delayed startup preserves newer resolution and FPS choices", flush=True
    )

    submitted = []
    reject_fps = [False]
    job = {
        "id": "output-test",
        "status": "rendering",
        "stage": "Rendering",
        "can_cancel": True,
        "log": "Rendering",
        "quality": "standard",
        "width": 1500,
        "height": 900,
        "resolution": "1500 × 900",
        "fps": 17.5,
        "name": "Custom output",
        "video": "/static/icon.svg",
        "created": "2026-01-01T00:00:00Z",
        "size": 100,
        "elapsed": 1,
    }

    def render(route):
        if reject_fps[0]:
            route.fulfill(
                status=422,
                json={
                    "detail": [
                        {
                            "loc": ["body", "fps"],
                            "msg": "Value error, Frame rate cannot be represented by MP4/H.264.",
                            "input": "PRIVATE INPUT",
                            "ctx": {"error": "PRIVATE CONTEXT"},
                        }
                    ]
                },
            )
            return
        submitted.append(route.request.post_data_json)
        route.fulfill(json=job)

    page.route("**/api/render", render)
    page.route("**/api/renders/output-test", lambda route: route.fulfill(json=job))
    page.evaluate("setOutputSettings({width:1500,height:900,fps:17.5})")
    page.locator("#render-button").click()
    expect(page.locator("#render-button span")).to_have_text("Cancel")
    expect(page.locator("#resolution-select")).to_be_disabled()
    expect(page.locator("#fps-select")).to_be_disabled()
    assert (
        submitted[-1]["width"],
        submitted[-1]["height"],
        submitted[-1]["fps"],
        submitted[-1]["quality"],
    ) == (1500, 900, 17.5, "standard")
    job.update(can_cancel=False, stage="Finishing render cleanup…")
    expect(page.locator("#render-button span")).to_have_text("Finishing…")
    expect(page.locator("#render-button")).to_be_disabled()
    expect(page.locator("#resolution-select")).to_be_disabled()
    expect(page.locator("#fps-select")).to_be_disabled()
    job.update(status="completed")
    expect(page.locator("#render-button")).to_have_attribute("data-state", "success")
    expect(page.locator("#resolution-select")).to_be_enabled()
    expect(page.locator("#fps-select")).to_be_enabled()
    expect(page.locator("#preview-badge")).to_have_text("1500 × 900 · 17.5 fps")
    print(
        "PASS: custom payload, render/cleanup locking, accurate preview metadata",
        flush=True,
    )

    held_scripts, held_analyze = [], []
    hold_script, hold_analyze = [False], [False]

    def script(route):
        if hold_script[0]:
            held_scripts.append(route)
        else:
            route.fulfill(body=SOURCE, content_type="text/plain")

    def analyze(route):
        if hold_analyze[0]:
            held_analyze.append(route)
        else:
            route.continue_()

    page.route("**/api/renders/output-test/script", script)
    page.route("**/api/analyze", analyze)
    page.evaluate("source=>setSource(source)", OTHER)
    page.locator("#higher-quality").click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-cancel").click()
    expect(page.locator("#resolution-select")).to_have_value("1500x900")
    expect(page.locator("#fps-select")).to_have_value("17.5")
    hold_script[0] = True
    page.locator("#higher-quality").click()
    wait_routes(held_scripts)
    page.locator("#resolution-select").select_option("3840x2160")
    page.locator("#fps-select").select_option("120")
    held_scripts.pop().fulfill(body=SOURCE, content_type="text/plain")
    expect(page.locator("#toast")).to_contain_text("workspace changed")
    expect(page.locator("#confirm-dialog")).to_be_hidden()
    hold_script[0] = False
    hold_script[0] = True
    page.locator("#higher-quality").click()
    wait_routes(held_scripts)
    page.locator("#fps-select").select_option("custom")
    page.locator("#custom-fps").fill("19.25")
    held_scripts.pop().fulfill(body=SOURCE, content_type="text/plain")
    page.wait_for_function("!window.higherQualityPending")
    expect(page.locator("#fps-dialog")).to_be_visible()
    expect(page.locator("#custom-fps")).to_have_value("19.25")
    expect(page.locator("#confirm-dialog")).to_be_hidden()
    page.evaluate("void startRender()")
    assert len(submitted) == 1
    page.keyboard.press("Escape")
    hold_script[0] = False
    count = len(submitted)
    for confirm in (False, True):
        page.evaluate("setOutputSettings({width:1500,height:900,fps:17.5})")
        page.evaluate("source=>setSource(source)", OTHER if confirm else SOURCE)
        hold_analyze[0] = True
        page.locator("#higher-quality").click()
        if confirm:
            expect(page.locator("#confirm-dialog")).to_be_visible()
            page.locator("#confirm-replace").click()
        wait_routes(held_analyze)
        page.evaluate("setOutputSettings({width:1080,height:1080,fps:25})")
        held_analyze.pop().fulfill(json={"scenes": ["MainScene"], "error": None})
        page.wait_for_function(
            "!window.higherQualityPending && analysis?.scenes.length"
        )
        expect(page.locator("#resolution-select")).to_have_value("1080x1080")
        expect(page.locator("#fps-select")).to_have_value("25")
        assert len(submitted) == count
        hold_analyze[0] = False
    print(
        "PASS: 1080p cancellation/stale fetch/delayed validation preserve output choices",
        flush=True,
    )
    page.evaluate("setOutputSettings({width:1500,height:900,fps:17.5})")
    page.evaluate("source=>setSource(source)", OTHER)
    hold_analyze[0] = True
    page.locator("#higher-quality").click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-replace").click()
    wait_routes(held_analyze)
    page.route(
        "**/api/renders/new-render",
        lambda route: route.fulfill(
            json={
                **job,
                "id": "new-render",
                "status": "rendering",
                "can_cancel": True,
                "stage": "New render",
            }
        ),
    )
    page.evaluate("beginTracking({id:'new-render',status:'rendering'})")
    held_analyze.pop().fulfill(json={"scenes": ["MainScene"], "error": None})
    page.wait_for_function("analysis?.scenes.length")
    expect(page.locator("#resolution-select")).to_have_value("1500x900")
    expect(page.locator("#fps-select")).to_have_value("17.5")
    expect(page.locator("#resolution-select")).to_be_disabled()
    expect(page.locator("#fps-select")).to_be_disabled()
    assert page.evaluate("activeJob.id") == "new-render"
    assert len(submitted) == count
    page.evaluate("activeJob=null;clearTimeout(pollTimer);updateRenderButton()")
    hold_analyze[0] = False
    print(
        "PASS: active tracking arriving during confirmed validation prevents later output mutation",
        flush=True,
    )

    reject_fps[0] = True
    page.evaluate("setOutputSettings({width:320,height:180,fps:.000001})")
    page.locator("#render-button").click()
    expect(page.locator("#validation-message")).to_contain_text(
        "Frame rate cannot be represented by MP4/H.264"
    )
    expect(page.locator("#problem-list")).to_contain_text(
        "Frame rate cannot be represented by MP4/H.264"
    )
    assert "PRIVATE" not in page.locator("body").inner_text()
    reject_fps[0] = False
    page.evaluate("setOutputSettings({width:1080,height:1080,fps:25})")
    assert len(validation_console) == 1
    print(
        "PASS: encoder boundary validation reports actionable API errors without input/context",
        flush=True,
    )

    root = Path(__file__).resolve().parents[1]
    page.add_script_tag(path=str(root / "node_modules/axe-core/axe.min.js"))
    for theme in ("dark", "light"):
        page.evaluate("theme=>applyTheme(theme)", theme)
        for width, height in ((1920, 1080), (1366, 768), (320, 800)):
            page.set_viewport_size({"width": width, "height": height})
            page.evaluate("switchPane('editor')")
            for selector in ("#resolution-select", "#fps-select"):
                rect = page.locator(selector).bounding_box()
                assert rect["x"] >= 0 and rect["x"] + rect["width"] <= width
                assert rect["width"] >= 80 and rect["height"] >= 32
                page.locator(selector).select_option("custom")
                result = page.evaluate(
                    "async()=>{const result=await axe.run(document,{runOnly:{type:'tag',values:['wcag2a','wcag2aa','wcag21a','wcag21aa']}});return result.violations.map(item=>({id:item.id,nodes:item.nodes.map(node=>node.target)}))}"
                )
                assert not result, result
                assert page.evaluate("document.documentElement.scrollWidth") <= width
                page.keyboard.press("Escape")
                expect(page.locator(selector)).to_be_focused()
            print(
                f"PASS custom-dialog WCAG AA/layout {theme} {width}x{height}",
                flush=True,
            )
    assert not errors, errors
    browser.close()
