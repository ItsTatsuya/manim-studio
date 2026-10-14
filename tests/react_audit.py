"""WCAG AA and viewport checks across the React workspace and settings."""

import re
from playwright.sync_api import expect
from browser_artifacts import artifact_directory
from browser_support import ROOT, api, browser_fixture, open_page, project


def audit():
    artifacts = artifact_directory()
    with browser_fixture() as (browser, url, _):
        context, page, errors = open_page(browser, url)
        value = project(page, url, "Accessibility scene")
        for theme in ("dark", "light"):
            api(page, url, "/api/preferences", {"values": {"manim-theme": theme}})
            for width, height in ((1920, 1080), (1366, 768), (320, 800)):
                page.set_viewport_size({"width": width, "height": height})
                for view in (
                    "preview",
                    "script",
                    "narration",
                    "assets",
                    "attachments",
                    "collapsed-sidebar",
                    "providers",
                    "general",
                    "voice",
                    "about",
                    "renders",
                    "onboarding-connect",
                    "onboarding-guide",
                    "onboarding-start",
                ):
                    route = (
                        f"/projects/{value['id']}"
                        if view.startswith("onboarding-")
                        or view
                        in (
                            "preview",
                            "script",
                            "narration",
                            "assets",
                            "attachments",
                            "collapsed-sidebar",
                        )
                        else "/renders"
                        if view == "renders"
                        else "/settings/" + {"voice": "narration"}.get(view, view)
                    )
                    page.goto(url + "/#" + route)
                    if view.startswith("onboarding-"):
                        api(
                            page,
                            url,
                            "/api/preferences",
                            {"values": {"studio-onboarding": ""}},
                        )
                        page.reload()
                        expect(
                            page.get_by_role("dialog", name="Welcome to Manim Studio")
                        ).to_be_visible()
                        for _ in range(
                            {
                                "onboarding-connect": 0,
                                "onboarding-guide": 1,
                                "onboarding-start": 2,
                            }[view]
                        ):
                            page.get_by_role(
                                "button", name="Continue", exact=True
                            ).click()
                    page.reload()
                    expect(page.locator("main")).to_be_visible()
                    if view in ("preview", "script", "narration", "assets"):
                        if not page.get_by_role(
                            "tab", name="Preview", exact=True
                        ).is_visible():
                            page.get_by_role(
                                "button", name="Show panel", exact=True
                            ).click()
                        page.get_by_role(
                            "tab", name=re.compile("^" + view.capitalize())
                        ).click()
                    elif view == "attachments":
                        if not page.get_by_role(
                            "button", name="Attach files", exact=True
                        ).is_visible():
                            page.get_by_role(
                                "button", name="Hide panel", exact=True
                            ).click()
                        page.get_by_role(
                            "button", name="Attach files", exact=True
                        ).click()
                        expect(
                            page.get_by_role(
                                "menuitem", name="Upload documents", exact=True
                            )
                        ).to_be_visible()
                    elif view == "collapsed-sidebar":
                        if width >= 768:
                            page.get_by_role(
                                "button", name="Collapse sidebar", exact=True
                            ).click()
                            expect(
                                page.get_by_role(
                                    "button", name="Expand sidebar", exact=True
                                )
                            ).to_be_visible()
                        else:
                            page.get_by_role(
                                "button", name="Projects", exact=True
                            ).click()
                            expect(
                                page.get_by_role("link", name="Settings", exact=True)
                            ).to_be_visible()
                    elif not view.startswith("onboarding-"):
                        expect(page.get_by_role("heading", level=1)).to_be_visible()
                    page.wait_for_function(
                        "dark => document.documentElement.classList.contains('dark') === dark",
                        arg=theme == "dark",
                    )
                    # Theme hydration can start color transitions after navigation.
                    # Audit the settled colors rather than an intermediate frame.
                    page.evaluate(
                        "async () => { await document.fonts.ready; await Promise.all(document.getAnimations().filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))); }"
                    )
                    page.add_script_tag(
                        path=str(ROOT / "node_modules/axe-core/axe.min.js")
                    )
                    result = page.evaluate(
                        """async () => (await axe.run(document, {runOnly: {type: 'tag', values: ['wcag2a','wcag2aa','wcag21a','wcag21aa']}})).violations.map(item => ({id: item.id, impact: item.impact, nodes: item.nodes.map(node => ({target: node.target, html: node.html, summary: node.failureSummary}))}))"""
                    )
                    if result:
                        page.screenshot(
                            path=str(artifacts / f"failure-{theme}-{width}-{view}.png"),
                            full_page=True,
                        )
                    assert not result, (theme, width, view, result)
                    assert (
                        page.evaluate("document.documentElement.scrollWidth") <= width
                    ), (theme, width, view, "horizontal overflow")
                    if view == "collapsed-sidebar":
                        api(
                            page,
                            url,
                            "/api/preferences",
                            {"values": {"studio-sidebar-collapsed": "false"}},
                        )
                    if view.startswith("onboarding-"):
                        bounds = page.get_by_role("dialog").bounding_box()
                        assert (
                            bounds["y"] >= 0
                            and bounds["y"] + bounds["height"] <= height
                        ), (theme, width, view, bounds)
                        page.get_by_role(
                            "button", name="Set up later", exact=True
                        ).click()
                        expect(page.get_by_role("dialog")).to_have_count(0)
                print(
                    f"PASS: WCAG AA and layout, 14 views, {theme} {width}×{height}",
                    flush=True,
                )
        page.set_viewport_size({"width": 1366, "height": 768})
        page.goto(url + f"/#/projects/{value['id']}")
        page.reload()
        expect(page.locator("main header").first).to_contain_text("Accessibility scene")
        if not page.get_by_role("tab", name="Preview", exact=True).is_visible():
            page.get_by_role("button", name="Show panel", exact=True).click()
        expect(page.get_by_role("tab", name="Preview", exact=True)).to_be_visible()
        page.get_by_role("tab", name="Script", exact=True).click()
        page.screenshot(path=str(artifacts / "react-workspace.png"))
        assert not errors, errors
        context.close()


if __name__ == "__main__":
    audit()
