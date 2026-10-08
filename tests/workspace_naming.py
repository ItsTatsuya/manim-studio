"""Animation naming and the conditional scene chooser stay usable and coherent."""

import os
import time
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

URL = os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765")
SINGLE = "from manim import *\nclass Orbit(Scene):\n    def construct(self):\n        self.wait(.1)\n"
MULTI = (
    SINGLE
    + "\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(.1)\n"
)

with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page(viewport={"width": 1366, "height": 768})
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.on(
        "console",
        lambda message: (
            errors.append(message.text) if message.type == "error" else None
        ),
    )
    page.on(
        "requestfailed",
        lambda request: (
            errors.append(f"{request.url}: {request.failure}")
            if request.failure != "net::ERR_ABORTED"
            else None
        ),
    )
    saved = {"manim-draft": "", "manim-tour-seen": "2", "manim-name": "Saved name"}
    held_preferences, held_analysis = [], []
    hold_preferences, hold_analysis = [False], [False]

    def wait_routes(routes):
        deadline = time.monotonic() + 5
        while not routes and time.monotonic() < deadline:
            page.wait_for_timeout(10)
        assert len(routes) == 1

    def preferences(route):
        if route.request.method == "GET":
            if hold_preferences[0]:
                held_preferences.append(route)
            else:
                route.fulfill(json=saved)
        else:
            saved.update(route.request.post_data_json["values"])
            route.fulfill(json=saved)

    def analyze(route):
        if hold_analysis[0]:
            held_analysis.append(route)
        else:
            route.continue_()

    page.route("**/api/preferences", preferences)
    page.route("**/api/analyze", analyze)
    page.route("**/api/renders", lambda route: route.fulfill(json=[]))
    page.route(
        "**/api/health",
        lambda route: route.fulfill(json={"manim": True, "latex": True}),
    )
    page.goto(URL)
    page.wait_for_function("window.codeEditor && preferencesReady")
    expect(page.locator(".scene-row")).to_be_hidden()
    expect(page.get_by_label("Animation name")).to_be_visible()
    assert page.locator("#animation-name").count() == 1
    assert page.locator(".topbar #animation-name").count() == 0
    assert page.locator("#render-settings #animation-name").count() == 1
    page.evaluate("source=>setSource(source)", SINGLE)
    expect(page.locator("#scene-select")).to_have_value("Orbit")
    expect(page.locator(".scene-row")).to_be_hidden()
    page.get_by_label("Animation name").fill("Orbit animation")
    submitted = []
    failed_job = {
        "id": "naming-test",
        "status": "failed",
        "log": "Test render finished",
    }

    def render(route):
        submitted.append(route.request.post_data_json)
        route.fulfill(json=failed_job)

    page.route("**/api/render", render)
    page.route(
        "**/api/renders/naming-test", lambda route: route.fulfill(json=failed_job)
    )
    page.locator("#render-button").click()
    expect(page.locator("#render-button")).to_have_attribute("data-state", "error")
    assert submitted[-1]["scene"] == "Orbit"
    assert submitted[-1]["name"] == "Orbit animation"
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-name"] == "Orbit animation"
    page.reload()
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    expect(page.get_by_label("Animation name")).to_have_value("Orbit animation")
    expect(page.locator("#scene-select")).to_have_value("Orbit")
    page.evaluate("showView('history');showView('workspace')")
    expect(page.get_by_label("Animation name")).to_have_value("Orbit animation")
    print(
        "PASS: one visible naming field, empty/single/non-MainScene payload and restoration",
        flush=True,
    )

    page.evaluate("source=>setSource(source)", MULTI)
    expect(page.locator(".scene-row")).to_be_visible()
    page.get_by_role("button", name="Scene: Orbit", exact=True).focus()
    page.keyboard.press("ArrowDown")
    page.keyboard.press("End")
    page.keyboard.press("Enter")
    expect(
        page.get_by_role("button", name="Scene: MainScene", exact=True)
    ).to_be_focused()
    page.keyboard.press("ArrowDown")
    page.keyboard.press("Home")
    page.keyboard.press("Enter")
    expect(page.locator("#scene-select")).to_have_value("Orbit")
    expect(page.get_by_role("button", name="Scene: Orbit", exact=True)).to_be_focused()
    page.get_by_label("Animation name").fill("Both scenes")
    before = page.locator("#resolution-select").bounding_box()["y"]
    hold_analysis[0] = True
    page.evaluate("source=>{void setSource(source)}", MULTI + "\n# edit\n")
    wait_routes(held_analysis)
    expect(page.locator(".scene-row")).to_be_visible()
    expect(page.locator("#scene-trigger")).to_be_disabled()
    assert page.locator("#resolution-select").bounding_box()["y"] == before
    held_analysis.pop().fulfill(json={"scenes": ["Orbit", "MainScene"], "error": None})
    expect(page.locator("#scene-trigger")).to_be_enabled()
    expect(page.locator("#scene-select")).to_have_value("Orbit")
    hold_analysis[0] = False
    for focus in ("trigger", "list"):
        page.evaluate("source=>setSource(source)", MULTI)
        page.locator("#scene-trigger").focus()
        if focus == "list":
            page.keyboard.press("ArrowDown")
            expect(page.locator("#scene-list")).to_be_visible()
        page.evaluate("source=>setSource(source)", SINGLE)
        expect(page.locator(".scene-row")).to_be_hidden()
        expect(page.get_by_label("Animation name")).to_be_focused()
        expect(page.locator("#scene-select")).to_have_value("Orbit")
    page.evaluate("source=>setSource(source)", MULTI)
    page.locator("#scene-trigger").focus()
    page.evaluate("void setSource('')")
    expect(page.locator(".scene-row")).to_be_hidden()
    expect(page.get_by_label("Animation name")).to_be_focused()
    expect(page.get_by_label("Animation name")).to_have_value("Both scenes")
    print(
        "PASS: multi-scene keyboard choice, validation stability and visible focus fallback",
        flush=True,
    )

    page.evaluate("async()=>{await persistPreferences()}")
    hold_preferences[0] = True
    page.reload()
    page.wait_for_function("window.codeEditor")
    page.get_by_label("Animation name").fill("New name while loading")
    wait_routes(held_preferences)
    held_preferences.pop().fulfill(json={**saved, "manim-name": "Stale name"})
    page.wait_for_function("preferencesReady")
    expect(page.get_by_label("Animation name")).to_have_value("New name while loading")
    hold_preferences[0] = False
    print(
        "PASS: delayed preferences retain the user's newer animation name", flush=True
    )

    root = Path(__file__).resolve().parents[1]
    page.add_script_tag(path=str(root / "node_modules/axe-core/axe.min.js"))
    for theme in ("dark", "light"):
        page.evaluate("theme=>applyTheme(theme)", theme)
        for width, height in ((1920, 1080), (1366, 768), (320, 800), (320, 600)):
            page.set_viewport_size({"width": width, "height": height})
            page.evaluate("switchPane('editor')")
            for source in (SINGLE, MULTI):
                page.evaluate("source=>setSource(source)", source)
                expect(page.locator("#render-button")).to_be_enabled()
                for selector in (
                    "#animation-name",
                    "#resolution-select",
                    "#fps-select",
                    "#render-button",
                ):
                    rect = page.locator(selector).bounding_box()
                    assert rect["x"] >= 0 and rect["x"] + rect["width"] <= width, (
                        selector,
                        rect,
                    )
                    assert rect["width"] >= 80 and rect["height"] >= 32, (
                        selector,
                        rect,
                    )
                    assert rect["y"] >= 0 and rect["y"] + rect["height"] <= height, (
                        selector,
                        rect,
                    )
                assert page.locator(".code-editor").bounding_box()["height"] >= 80
                assert page.locator(".topbar h1").bounding_box()["width"] > 100
                assert page.evaluate("document.documentElement.scrollWidth") <= width
                # Audit settled control colors, rather than disabled-to-ready transitions.
                page.evaluate(
                    "async()=>{await new Promise(requestAnimationFrame);await Promise.all(document.getAnimations().filter(animation=>animation.effect.getTiming().iterations!==Infinity).map(animation=>animation.finished.catch(()=>{})))}"
                )
                result = page.evaluate(
                    "async()=>{const result=await axe.run(document,{runOnly:{type:'tag',values:['wcag2a','wcag2aa','wcag21a','wcag21aa']}});return result.violations.map(item=>({id:item.id,nodes:item.nodes.map(node=>node.target)}))}"
                )
                assert not result, result
            print(
                f"PASS naming/scene WCAG AA/layout {theme} {width}x{height}", flush=True
            )
    assert not errors, errors
    browser.close()
