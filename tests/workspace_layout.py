"""Capture and verify the core workspace at the required viewport sizes."""

import os
import sys
from playwright.sync_api import sync_playwright
from browser_artifacts import artifact_directory

artifacts = artifact_directory()
stage = sys.argv[1] if len(sys.argv) > 1 else "final"
with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    page = browser.new_page()
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    for width, height in ((1920, 1080), (1366, 768)):
        page.set_viewport_size({"width": width, "height": height})
        page.goto(os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765"))
        page.wait_for_function("window.engineHealth?.manim")
        page.evaluate("closeTour(false)")
        page.evaluate("document.fonts.ready")
        measurements = page.evaluate("""() => ({
            width:innerWidth,height:innerHeight,scrollWidth:document.documentElement.scrollWidth,
            scrollHeight:document.documentElement.scrollHeight,
            panels:Object.fromEntries(['#editor-panel','#preview-panel','#render-button','.code-editor'].map(selector=>{
                const element=document.querySelector(selector),rect=element.getBoundingClientRect();
                return [selector,{top:rect.top,bottom:rect.bottom,left:rect.left,right:rect.right,height:rect.height,visible:!!rect.width&&!!rect.height}];
            }))
        })""")
        assert measurements["scrollHeight"] <= height, measurements
        assert measurements["scrollWidth"] <= width, measurements
        for selector, rect in measurements["panels"].items():
            assert (
                rect["visible"]
                and rect["top"] >= 0
                and rect["bottom"] <= height
                and rect["right"] <= width
            ), (selector, measurements)
        assert measurements["panels"][".code-editor"]["height"] >= 150, measurements
        output = artifacts / f"workspace-{stage}-{width}.png"
        page.screenshot(path=str(output))
        print(
            f"PASS {stage} {width}x{height}: no page scroll; editor, preview and Render visible",
            flush=True,
        )
    assert not errors, errors
    browser.close()
