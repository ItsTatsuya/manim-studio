"""Capture the real workspace after rendering a LaTeX and Typst demo."""

import argparse
import os
from pathlib import Path

from playwright.sync_api import expect, sync_playwright

SOURCE = """from manim import *

class EquationDemo(Scene):
    def construct(self):
        self.camera.background_color = "#101014"
        title = Text("One identity, two engines", font_size=36)
        title.to_edge(UP)

        latex = VGroup(
            Text("LaTeX", font_size=24, color=BLUE),
            MathTex(r"e^{i\\pi} + 1 = 0", font_size=64),
        ).arrange(DOWN, buff=0.3)
        typst = VGroup(
            Text("Typst", font_size=24, color=GREEN),
            MathTypst("e^(i pi) + 1 = 0", font_size=64),
        ).arrange(DOWN, buff=0.3)
        VGroup(latex, typst).arrange(DOWN, buff=0.7)

        self.play(FadeIn(title), Write(latex), run_time=1)
        self.play(Write(typst), run_time=1)
        self.wait(1)
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8765")
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "docs/images/manim-studio.png",
    )
    args = parser.parse_args()
    if os.environ.get("MANIM_STUDIO_UPDATE_SCREENSHOT") != "1":
        parser.error(
            "Set MANIM_STUDIO_UPDATE_SCREENSHOT=1 to replace the product image"
        )
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        page = browser.new_page(viewport={"width": 1440, "height": 900})
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.goto(args.url)
        page.wait_for_function(
            "window.codeEditor && preferencesReady && window.engineHealth?.typst"
        )
        page.evaluate("closeTour(false);applyTheme('dark')")
        page.evaluate("source => setSource(source, 'Euler identity')", SOURCE)
        expect(page.locator("#render-button")).to_be_enabled()
        page.locator("#resolution-select").select_option("1280x720")
        page.locator("#fps-select").select_option("30")
        page.locator("#render-button").click()
        page.wait_for_function(
            "document.querySelector('#render-button').dataset.state === 'success'",
            timeout=120000,
        )
        page.wait_for_function(
            "document.querySelector('#video-player').readyState >= 2"
        )
        page.locator("#problems-tab").click()
        if "collapsed" not in (page.locator("#log-panel").get_attribute("class") or ""):
            page.locator("#drawer-toggle").click()
        expect(page.locator("#toast")).not_to_be_visible(timeout=10000)
        page.evaluate("""async () => {
            await document.fonts.ready;
            const video = document.querySelector('#video-player');
            video.pause();
            await new Promise(resolve => {
                video.addEventListener('seeked', resolve, {once: true});
                video.currentTime = Math.max(0, video.duration - 0.1);
            });
        }""")
        assert page.evaluate("document.documentElement.scrollHeight <= innerHeight")
        assert not errors, errors
        args.output.parent.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=str(args.output))
        print(
            "PASS: real LaTeX and Typst render, visible editor/preview/controls:",
            args.output,
        )
        browser.close()


if __name__ == "__main__":
    main()
