"""WCAG AA and viewport checks across the React workspace and settings."""

import re
from playwright.sync_api import expect
from browser_artifacts import artifact_directory
from browser_support import ROOT, api, browser_fixture, open_page, project, wait_api


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
                    "rail-projects",
                    "providers",
                    "general",
                    "voice",
                    "about",
                    "renders",
                    "folders",
                    "batch",
                    "usage",
                    "onboarding-connect",
                    "onboarding-voice",
                    "onboarding-guide",
                    "onboarding-start",
                ):
                    if view == "rail-projects" and width < 768:
                        continue
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
                            "rail-projects",
                            "folders",
                            "batch",
                            "usage",
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
                                "onboarding-voice": 1,
                                "onboarding-guide": 2,
                                "onboarding-start": 3,
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
                    elif view in ("collapsed-sidebar", "rail-projects"):
                        if width >= 768:
                            page.get_by_role(
                                "button", name="Collapse sidebar", exact=True
                            ).click()
                            expect(
                                page.get_by_role(
                                    "button", name="Expand sidebar", exact=True
                                )
                            ).to_be_visible()
                            if view == "rail-projects":
                                page.get_by_role(
                                    "button", name="Switch project", exact=True
                                ).click()
                                expect(
                                    page.get_by_role(
                                        "dialog", name="Projects", exact=True
                                    )
                                ).to_be_visible()
                        else:
                            page.get_by_role(
                                "button", name="Projects", exact=True
                            ).click()
                            expect(
                                page.get_by_role("link", name="Settings", exact=True)
                            ).to_be_visible()
                    elif view in ("folders", "batch", "usage"):
                        if width < 768:
                            page.get_by_role(
                                "button", name="Projects", exact=True
                            ).click()
                        if view == "usage":
                            page.route(
                                "**/api/usage?*",
                                lambda route: route.fulfill(
                                    json={
                                        "updated": 1792070000,
                                        "providers": [
                                            {
                                                "id": "codex",
                                                "name": "Codex",
                                                "available": True,
                                                "plan": "plus",
                                                "error": None,
                                                "windows": [
                                                    {
                                                        "id": "weekly",
                                                        "label": "Weekly",
                                                        "used_percent": 39,
                                                        "left_percent": 61,
                                                        "resets_at": 1792652259,
                                                    }
                                                ],
                                            }
                                        ],
                                    }
                                ),
                            )
                        page.get_by_role(
                            "button",
                            name={
                                "folders": "Select folders",
                                "batch": "Videos from PDFs",
                                "usage": "Usage limits",
                            }[view],
                            exact=True,
                        ).click()
                        expect(page.get_by_role("dialog")).to_be_visible()
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
                    if view in ("collapsed-sidebar", "rail-projects"):
                        if width >= 768:
                            # The app saves the collapse after a short delay; let it
                            # land first or it overwrites the reset below.
                            wait_api(
                                page,
                                url,
                                "/api/preferences",
                                lambda prefs: (
                                    prefs.get("studio-sidebar-collapsed") == "true"
                                ),
                            )
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
                    f"PASS: WCAG AA and layout, {19 if width >= 768 else 18} views, {theme} {width}×{height}",
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


def audit_recovery():
    artifacts = artifact_directory()
    with browser_fixture() as (browser, url, _):
        context, page, errors = open_page(browser, url)
        value = project(page, url, "Recovery accessibility")
        original = api(page, url, f"/api/projects/{value['id']}")["source"]
        video_state, render_state = [], []
        page.route(
            "**/api/videos",
            lambda route: route.fulfill(
                json=video_state, content_type="application/json"
            ),
        )
        page.route(
            "**/api/renders",
            lambda route: route.fulfill(
                json=render_state, content_type="application/json"
            ),
        )
        for theme in ("dark", "light"):
            for width in (1366, 320):
                page.set_viewport_size({"width": width, "height": 800})
                for state in (
                    "script-error",
                    "repairing",
                    "paused",
                    "queue",
                    "render-error",
                ):
                    source = (
                        "from manim import *\nclass Broken(Scene)\n    pass"
                        if state == "script-error"
                        else original
                    )
                    api(page, url, f"/api/projects/{value['id']}", {"source": source})
                    api(
                        page,
                        url,
                        "/api/preferences",
                        {
                            "values": {
                                "manim-theme": theme,
                                "studio-layout": '{"width":620,"open":true,"tab":"preview"}',
                            }
                        },
                    )
                    video_state = (
                        []
                        if state in {"script-error", "render-error"}
                        else [
                            {
                                "id": "recovery-audit",
                                "project": value["id"],
                                "workspace": None,
                                "thread": None,
                                "name": value["name"],
                                "status": "failed"
                                if state == "paused"
                                else "repairing",
                                "stage": "Automatic repair paused"
                                if state == "paused"
                                else "Fixing the video · attempt 1 of 3",
                                "attempt": 3 if state == "paused" else 1,
                                "max_attempts": 3,
                                "render": None,
                                "error": "The video still needs a fix after 3 attempts. Line 4: ValueError: the animation needs a valid object.",
                                "updated": "2030-01-01T00:00:00+00:00",
                            }
                        ]
                    )
                    render_state = (
                        [
                            {
                                "id": "0" * 32,
                                "project": value["id"],
                                "name": value["name"],
                                "scene": "MainScene",
                                "quality": "preview",
                                "status": "failed",
                                "created": "2026-01-01T00:00:00+00:00",
                                "error": "The script could not render.",
                                "stage": "Render needs attention",
                            }
                        ]
                        if state == "render-error"
                        else []
                    )
                    path = (
                        "/renders"
                        if state in {"queue", "render-error"}
                        else f"/projects/{value['id']}"
                    )
                    page.goto(url + "/#" + path)
                    page.reload()
                    if state in {"script-error", "paused", "render-error"}:
                        expect(
                            page.get_by_role(
                                "button", name="Send to LLM", exact=True
                            ).first
                        ).to_be_visible()
                    elif state == "queue":
                        expect(
                            page.get_by_role(
                                "button",
                                name="Stop video Recovery accessibility",
                                exact=True,
                            )
                        ).to_be_visible()
                    else:
                        expect(
                            page.get_by_text(
                                "Fixing the video · attempt 1 of 3", exact=True
                            ).first
                        ).to_be_visible()
                    page.evaluate(
                        "async () => { await document.fonts.ready; await Promise.all(document.getAnimations().filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))); }"
                    )
                    page.add_script_tag(
                        path=str(ROOT / "node_modules/axe-core/axe.min.js")
                    )
                    result = page.evaluate(
                        "async () => (await axe.run(document, {runOnly:{type:'tag',values:['wcag2a','wcag2aa','wcag21a','wcag21aa']}})).violations.map(v=>({id:v.id,nodes:v.nodes.map(n=>({target:n.target,summary:n.failureSummary}))}))"
                    )
                    page.screenshot(
                        path=str(artifacts / f"recovery-{state}-{theme}-{width}.png"),
                        full_page=True,
                    )
                    assert not result, (state, theme, width, result)
                    assert (
                        page.evaluate("document.documentElement.scrollWidth") <= width
                    ), (state, theme, width, "horizontal overflow")
                print(
                    f"PASS: recovery controls, 5 states, {theme}, {width}px", flush=True
                )
        assert not errors, errors
        context.close()


if __name__ == "__main__":
    audit()
    audit_recovery()
