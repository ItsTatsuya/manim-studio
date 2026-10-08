"""Keyboard guards, literal-safe Format and preserving a user's log position."""

import os
from playwright.sync_api import sync_playwright, expect

with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page(viewport={"width": 1366, "height": 768})
    page.goto(os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765"))
    page.wait_for_function("window.codeEditor && window.engineHealth?.manim")
    page.evaluate(
        "source=>{closeTour(false); window.engineHealth.latex=false; return setSource(source)}",
        'from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.add(MathTex("x"))',
    )
    expect(page.locator("#render-button")).to_be_disabled()
    requests = []
    page.on(
        "request",
        lambda request: (
            requests.append(request.url)
            if request.method == "POST" and request.url.endswith("/api/render")
            else None
        ),
    )
    page.keyboard.press("Control+Enter")
    page.wait_for_timeout(300)
    assert not requests, "Unavailable LaTeX must also block the keyboard path"
    page.evaluate("window.engineHealth.latex=true")
    source = 'from manim import *\nvalue = """keep trailing spaces  \n\tkeep tabs  \nend"""\nclass MainScene(Scene):\n        def construct(self):\n                self.wait(.2)\n'
    page.evaluate("source=>setSource(source)", source)
    page.locator("#format-script").click()
    expect(page.locator("#toast")).to_contain_text("Code whitespace normalized")
    formatted = page.evaluate("codeEditor.getSource()")
    assert '"""keep trailing spaces  \n\tkeep tabs  \nend"""' in formatted
    assert "\n    def construct" in formatted

    count = [500]

    def job_response(route):
        count[0] += 10
        route.fulfill(
            json={
                "id": "scroll-test",
                "status": "rendering",
                "stage": "Animation 1 · 50%",
                "elapsed": 1,
                "created": "2026-01-01T00:00:00Z",
                "log": "\n".join(f"Render line {index}" for index in range(count[0])),
            }
        )

    page.route("**/api/renders/scroll-test", job_response)
    page.evaluate('beginTracking({id:"scroll-test", status:"rendering"})')
    expect(page.locator("#render-log")).to_contain_text("Render line 509")
    page.evaluate('document.querySelector("#drawer-body").scrollTop=0')
    expect(page.locator("#render-log")).to_contain_text("Render line 519")
    assert page.evaluate('document.querySelector("#drawer-body").scrollTop') == 0
    page.evaluate(
        'const d=document.querySelector("#drawer-body");d.scrollTop=d.scrollHeight'
    )
    expect(page.locator("#render-log")).to_contain_text("Render line 529")
    assert page.evaluate(
        'const d=document.querySelector("#drawer-body"); d.scrollHeight-d.scrollTop-d.clientHeight < 50'
    )
    page.evaluate("activeJob=null;clearTimeout(pollTimer)")
    browser.close()
    print(
        "PASS: Ctrl+Enter dependency guard, literal-safe Format, and log scroll preservation",
        flush=True,
    )
