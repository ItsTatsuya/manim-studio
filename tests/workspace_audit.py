"""Automated WCAG audit plus dependency and higher-quality re-render behavior."""

from pathlib import Path
import os
from playwright.sync_api import sync_playwright, expect
from browser_artifacts import artifact_directory

root = Path(__file__).resolve().parents[1]
artifacts = artifact_directory()
with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page()
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
    page.goto(os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765"))
    page.wait_for_function("window.codeEditor && window.engineHealth?.manim")
    page.evaluate("closeTour(false);setSource(EXAMPLES[0].source)")
    page.wait_for_function('!document.querySelector("#render-button").disabled')
    page.add_script_tag(path=str(root / "node_modules/axe-core/axe.min.js"))
    for theme in ("dark", "light"):
        page.evaluate("(theme)=>applyTheme(theme)", theme)
        for width, height in ((1920, 1080), (1366, 768), (320, 800)):
            page.set_viewport_size({"width": width, "height": height})
            page.evaluate('switchPane("editor")')
            result = page.evaluate(
                """async () => {const results=await axe.run(document,{runOnly:{type:'tag',values:['wcag2a','wcag2aa','wcag21a','wcag21aa']}});return results.violations.map(item=>({id:item.id,impact:item.impact,nodes:item.nodes.map(node=>({target:node.target,summary:node.failureSummary}))}))}"""
            )
            assert not result, result
            print(f"PASS axe WCAG AA {theme} {width}x{height}", flush=True)
    page.set_viewport_size({"width": 1366, "height": 768})
    page.evaluate(
        'applyTheme("dark");setSource("```python\\nfrom manim import *\\nclass MainScene(Scene)\\n    pass\\n```")'
    )
    expect(page.locator("#problem-list")).to_contain_text("Line 3")
    page.evaluate(
        "(source)=>{window.engineHealth.latex=false;setSource(source)}",
        'from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.add(MathTex("x"))',
    )
    expect(page.locator("#problem-list")).to_contain_text("repair equation tools")
    expect(page.locator("#render-button")).to_be_disabled()
    page.evaluate("window.engineHealth.latex=true;setSource(EXAMPLES[0].source)")
    expect(page.locator("#render-button")).to_be_enabled()
    page.locator("#resolution-select").select_option("854x480")
    page.locator("#fps-select").select_option("15")
    page.locator("#render-button").click()
    page.wait_for_function(
        'document.querySelector("#render-button").dataset.state==="success"',
        timeout=60000,
    )
    page.locator("#save-library").click()
    expect(page.locator("#toast")).to_contain_text("Saved in My renders")
    rendered_id = page.evaluate("lastCompletedJob.id")
    script_response = page.request.get(
        f"{page.url.rstrip('/')}/api/renders/{rendered_id}/script"
    )
    assert script_response.ok
    rendered_source = script_response.text()
    if os.name == "nt":
        assert "\r\n" in rendered_source, (
            "Exercise Windows persisted script line endings"
        )
    page.evaluate(
        'setSource("from manim import *\\nclass Different(Scene):\\n    def construct(self):\\n        self.wait(.2)")'
    )
    expect(page.locator("#render-button")).to_be_enabled()
    page.locator("#higher-quality").click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-cancel").click()
    assert "class Different" in page.evaluate("codeEditor.getSource()")
    page.locator("#higher-quality").click()
    page.locator("#confirm-replace").click()
    page.wait_for_function(
        'document.querySelector("#render-button").dataset.state==="success"',
        timeout=60000,
    )
    expect(page.locator("#video-meta")).to_contain_text("60fps")
    assert "class MainScene" in page.evaluate("codeEditor.getSource()")
    assert page.evaluate("codeEditor.getSource()") == rendered_source.replace(
        "\r\n", "\n"
    ).replace("\r", "\n")
    print(
        "PASS: fenced-code line mapping, missing-LaTeX hint, verified library save and higher-quality source restoration",
        flush=True,
    )
    # Capture the actual end state after seeking a representative frame.
    page.evaluate(
        'document.querySelector("#toast").hidden=true;const v=document.querySelector("#video-player");v.currentTime=2.5'
    )
    page.wait_for_function('document.querySelector("#video-player").readyState>=2')
    page.evaluate(
        'async()=>{const v=document.querySelector("#video-player");await v.play();await new Promise(resolve=>setTimeout(resolve,600));v.pause();}'
    )
    page.evaluate('selectDrawer("problems")')
    for width, height in ((1920, 1080), (1366, 768)):
        page.set_viewport_size({"width": width, "height": height})
        page.screenshot(path=str(artifacts / f"workspace-finished-{width}.png"))
        if width == 1920 and os.environ.get("MANIM_STUDIO_UPDATE_SCREENSHOT") == "1":
            target = root / "docs/images/manim-studio.png"
            target.parent.mkdir(parents=True, exist_ok=True)
            page.screenshot(path=str(target))
    assert not errors, errors
    browser.close()
