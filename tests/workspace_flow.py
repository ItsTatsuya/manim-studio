"""Exercise the real editor, render API and preview in the revised workspace."""

from pathlib import Path
import os
from playwright.sync_api import sync_playwright, expect

root = Path(__file__).resolve().parents[1]
with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    context = browser.new_context(
        viewport={"width": 1366, "height": 768},
        permissions=["clipboard-read", "clipboard-write"],
    )
    page = context.new_page()
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
            errors.append(f"{request.method} {request.url}: {request.failure}")
            if request.failure != "net::ERR_ABORTED"
            else None
        ),
    )
    page.goto(os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765"))
    page.wait_for_function("window.codeEditor && window.engineHealth?.manim")
    page.evaluate('closeTour(false);setSource("")')
    # Clipboard import and the actual contenteditable editor.
    source = page.evaluate("EXAMPLES[0].source")
    page.evaluate("(source)=>navigator.clipboard.writeText(source)", source)
    page.locator("#paste-button").click()
    expect(page.locator("#render-button")).to_be_enabled()
    assert page.evaluate("codeEditor.getSource()") == source
    expect(page.locator(".scene-row")).to_be_hidden()
    expect(page.locator("#scene-select")).to_have_value("MainScene")
    expect(page.locator("#scene-trigger")).to_be_hidden()
    expect(page.locator("#editor-empty")).to_be_hidden()
    page.locator("#font-increase").click()
    assert (
        page.locator(".cm-editor").evaluate("(e)=>getComputedStyle(e).fontSize")
        == "15px"
    )
    page.locator("#font-decrease").click()
    page.locator("#wrap-toggle").click()
    expect(page.locator("#wrap-toggle")).to_have_attribute("aria-pressed", "true")
    page.locator("#find-button").click()
    expect(page.locator(".cm-search")).to_be_visible()
    page.locator(".cm-search [name=search]").fill("Circle")
    page.locator(".cm-search [name=close]").click()
    page.evaluate(
        'setSource("from manim import *\\nclass MainScene(Scene):\\n    def construct(self):\\n        Cre")'
    )
    page.locator(".cm-content").click()
    page.keyboard.press("Control+End")
    page.keyboard.press("Control+Space")
    expect(page.locator(".cm-tooltip-autocomplete")).to_be_visible()
    expect(page.locator(".cm-tooltip-autocomplete")).to_contain_text("Create")
    page.keyboard.press("Escape")
    page.evaluate(
        "(source)=>setSource(source)",
        "from manim import *\nclass MainScene(Scene):\n        def construct(self):\n                self.wait(.2)\n",
    )
    page.locator(".cm-content").focus()
    page.keyboard.press("Control+End")
    page.locator("#format-script").click()
    expect(page.locator("#toast")).to_contain_text("Code whitespace normalized")
    assert "    def construct" in page.evaluate("codeEditor.getSource()")
    assert "                self.wait" not in page.evaluate("codeEditor.getSource()")
    page.locator(".cm-content").focus()
    page.keyboard.press("Escape")
    page.keyboard.press("Tab")
    assert page.evaluate('!document.activeElement.classList.contains("cm-content")')
    print(
        "PASS: Format handles shrinking indentation; Escape + Tab exits the editor",
        flush=True,
    )
    print(
        "PASS: Paste, CodeMirror editing, font size, wrap, find and Manim autocomplete",
        flush=True,
    )
    # AST errors surface as line diagnostics; clicking a problem moves selection.
    page.evaluate(
        'setSource("from manim import *\\nclass MainScene(Scene)\\n    pass")'
    )
    expect(page.locator("#problem-list")).to_contain_text("Line 2")
    expect(page.locator(".cm-lintRange-error")).to_be_visible()
    page.locator("#problem-list button").click()
    assert (
        page.evaluate(
            "codeEditor.view.state.doc.lineAt(codeEditor.view.state.selection.main.head).number"
        )
        == 2
    )
    expect(page.locator("#render-button")).to_be_disabled()
    # Native import input and a keyboard-operated multi-scene select.
    multiple = "from manim import *\nclass First(Scene):\n    def construct(self):\n        self.wait(.2)\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(.2)"
    page.locator("#file-input").set_input_files(
        {"name": "scenes.py", "mimeType": "text/x-python", "buffer": multiple.encode()}
    )
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-replace").click()
    expect(page.locator("#scene-trigger")).to_be_visible()
    page.locator("#scene-trigger").focus()
    page.keyboard.press("ArrowDown")
    page.keyboard.press("Home")
    page.keyboard.press("Enter")
    assert page.evaluate('document.querySelector("#scene-select").value') == "First"
    print(
        "PASS: syntax underline, clickable Problems, Import file and keyboard scene selector",
        flush=True,
    )
    page.evaluate("setSource(EXAMPLES[0].source,EXAMPLES[0].title)")
    expect(page.locator("#render-button")).to_be_enabled()
    page.locator("#resolution-select").select_option("854x480")
    page.locator("#fps-select").select_option("15")
    page.keyboard.press("Control+Enter")
    expect(page.locator("#render-button span")).to_have_text("Cancel")
    expect(page.locator("#render-button")).to_be_enabled()
    expect(page.locator("#bar-progress-state")).to_have_count(0)
    expect(page.locator("#render-overlay")).to_be_visible()
    page.wait_for_function(
        'document.querySelector("#render-button").dataset.state === "success"',
        timeout=60000,
    )
    expect(page.locator("#video-actions")).to_be_visible()
    page.wait_for_function('document.querySelector("#video-player").readyState>=2')
    assert page.evaluate('document.querySelector("#video-player").error') is None
    expect(page.locator("#video-meta")).to_contain_text("Rendered in")
    page.locator("#loop-video").check()
    page.locator("#playback-speed").select_option("1.5")
    assert page.evaluate(
        'document.querySelector("#video-player").loop && document.querySelector("#video-player").playbackRate===1.5'
    )
    page.evaluate("showVideo(lastCompletedJob)")
    page.wait_for_function('document.querySelector("#video-player").readyState>=2')
    assert page.evaluate('document.querySelector("#video-player").playbackRate') == 1.5
    assert (
        page.locator("#download-video").get_attribute("href").endswith("?download=true")
    )
    print(
        "PASS: real Ctrl+Enter render, persistent bar, progress, playback, loop, speed and export",
        flush=True,
    )
    broken = 'from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.add(Circle())\n        raise ValueError("Workspace verification error")'
    page.evaluate("(source)=>setSource(source)", broken)
    expect(page.locator("#render-button")).to_be_enabled()
    page.locator("#render-button").click()
    expect(page.locator("#error-card")).to_be_visible(timeout=60000)
    expect(page.locator("#problem-list")).to_contain_text(
        "Workspace verification error"
    )
    expect(page.locator("#problem-list")).to_contain_text("Line 5")
    page.locator("#problem-list button").click()
    assert (
        page.evaluate(
            "codeEditor.view.state.doc.lineAt(codeEditor.view.state.selection.main.head).number"
        )
        == 5
    )
    expect(page.locator("#render-button")).to_have_attribute("data-state", "error")
    page.locator("#log-tab").click()
    expect(page.locator("#render-log")).to_contain_text("ValueError")
    page.locator("#problems-tab").click()
    page.locator("#copy-error").click()
    print(
        "PASS: runtime error maps to line 5, Render log and copyable recovery details",
        flush=True,
    )
    page.locator("[data-view=history]").click()
    expect(page.locator(".history-card").first).to_be_visible()
    page.locator(".history-card button").first.click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-cancel").click()
    assert page.evaluate("editor.value") == broken
    page.evaluate("setSource(EXAMPLES[0].source)")
    expect(page.locator("#render-button")).to_be_enabled()
    page.reload()
    page.wait_for_function(
        'window.codeEditor && codeEditor.getSource().includes("class MainScene")'
    )
    print(
        "PASS: history navigation, replace confirmation and restored autosaved draft",
        flush=True,
    )
    assert not errors, errors
    browser.close()
