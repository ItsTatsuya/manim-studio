"""Check short/narrow layouts, tour placement, divider bounds and cancellation."""

import os
from playwright.sync_api import sync_playwright, expect
from browser_artifacts import artifact_directory

artifacts = artifact_directory()
with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page()
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765"))
    page.wait_for_function("window.codeEditor && window.engineHealth?.manim")
    page.evaluate("closeTour(false);setSource(EXAMPLES[0].source)")
    page.wait_for_function('!document.querySelector("#render-button").disabled')
    page.set_viewport_size({"width": 900, "height": 800})
    for tab, first, last in (
        ("#editor-tab", "#editor-tab", "#preview-tab"),
        ("#problems-tab", "#problems-tab", "#log-tab"),
    ):
        page.evaluate('switchPane("editor")')
        page.locator(tab).focus()
        page.keyboard.press("Home")
        expect(page.locator(first)).to_be_focused()
        page.keyboard.press("Home")
        expect(page.locator(first)).to_be_focused()
        page.keyboard.press("End")
        expect(page.locator(last)).to_be_focused()
        page.keyboard.press("End")
        expect(page.locator(last)).to_be_focused()
    print("PASS: Home and End reliably select the first and last tabs", flush=True)
    for width, height in (
        (1920, 1080),
        (1366, 768),
        (1100, 768),
        (1099, 768),
        (900, 800),
        (690, 800),
        (430, 800),
        (320, 800),
        (320, 600),
    ):
        page.set_viewport_size({"width": width, "height": height})
        page.evaluate('switchPane("editor")')
        bounds = page.evaluate("""() => ({w:innerWidth,h:innerHeight,scrollWidth:document.documentElement.scrollWidth,
            render:document.querySelector('#render-button').getBoundingClientRect().toJSON(),
            editor:document.querySelector('.code-editor').getBoundingClientRect().toJSON()})""")
        assert bounds["scrollWidth"] <= width, bounds
        assert bounds["render"]["bottom"] <= height and bounds["render"]["top"] >= 0, (
            bounds
        )
        assert bounds["editor"]["height"] >= 80, bounds
        if height < 740 and width < 700:
            drawer = page.locator(".drawer-heading").bounding_box()
            panel = page.locator("#editor-panel").bounding_box()
            assert drawer["y"] + drawer["height"] <= panel["y"] + panel["height"], (
                drawer,
                panel,
            )
        if width < 1100:
            page.locator("#preview-tab").click()
            expect(page.locator("#preview-panel")).to_be_visible()
            expect(page.locator("#editor-panel")).to_be_hidden()
            page.locator("#editor-tab").click()
        page.screenshot(
            path=str(artifacts / f"workspace-responsive-{width}-{height}.png")
        )
        print(f"PASS responsive {width}x{height}", flush=True)
    # Keyboard divider is clamped after shrinking the window.
    page.set_viewport_size({"width": 1920, "height": 1080})
    page.locator("#split-divider").focus()
    page.keyboard.press("End")
    assert int(page.locator("#split-divider").get_attribute("aria-valuenow")) == 70
    page.set_viewport_size({"width": 1100, "height": 768})
    page.wait_for_timeout(100)
    assert page.evaluate("document.documentElement.scrollWidth") <= 1100
    assert page.locator("#preview-panel").bounding_box()["width"] >= 320
    # All seven non-modal callouts stay onscreen on desktop and narrow screens.
    for width in (1366, 690, 320):
        page.set_viewport_size({"width": width, "height": 800})
        page.evaluate("openTour(false)")
        for _ in range(7):
            bounds = page.evaluate(
                """() => { const p=document.querySelector('#tour-popover').getBoundingClientRect(),t=document.querySelector('.tour-focus').getBoundingClientRect();return {inside:p.left>=0&&p.top>=0&&p.right<=innerWidth&&p.bottom<=innerHeight,separate:p.right<=t.left||p.left>=t.right||p.bottom<=t.top||p.top>=t.bottom,step:tourStep}; }"""
            )
            assert bounds["inside"] and bounds["separate"], (width, bounds)
            page.locator("#tour-next").click()
    print(
        "PASS: divider minimums, keyboard resize and seven contextual tour steps",
        flush=True,
    )
    for theme in ("dark", "light"):
        page.evaluate("theme=>applyTheme(theme)", theme)
        for width, height in ((1366, 768), (320, 600)):
            page.set_viewport_size({"width": width, "height": height})
            page.evaluate(
                'setSource("from manim import *\\nclass MainScene(Scene):\\n    def construct(self):\\n        self.play(Create(Circle()),run_time=120)")'
            )
            expect(page.locator("#render-button")).to_be_enabled()
            page.locator("#render-button").click()
            expect(page.locator("#bar-progress-state")).to_have_count(0)
            expect(page.locator("#render-overlay")).to_be_visible()
            expect(page.locator("#render-button span")).to_have_text("Cancel")
            expect(page.locator("#render-button")).to_be_enabled()
            assert page.locator("#render-button").bounding_box()["y"] + 44 <= height
            page.locator("#render-button").focus()
            page.keyboard.press("Enter")
            expect(page.locator("#render-overlay")).to_be_hidden(timeout=30000)
            expect(page.locator("#render-button")).to_have_attribute(
                "data-state", "idle"
            )
    print(
        "PASS: progress and Cancel stay visible; cancelled render returns to idle",
        flush=True,
    )
    assert not errors, errors
    browser.close()
