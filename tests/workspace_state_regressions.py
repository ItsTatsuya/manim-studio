"""Delayed requests, session-save failures and keyboard state regression checks."""

import os
import time
from playwright.sync_api import sync_playwright, expect


SOURCE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.wait(.1)\n"
OTHER = SOURCE.replace("MainScene", "OtherScene")
URL = os.environ.get("MANIM_STUDIO_URL", "http://127.0.0.1:8765")


with sync_playwright() as playwright:
    browser = playwright.chromium.launch()
    context = browser.new_context(viewport={"width": 1366, "height": 768})
    page = context.new_page()

    def wait_for_routes(routes, count):
        deadline = time.monotonic() + 5
        while len(routes) < count and time.monotonic() < deadline:
            page.wait_for_timeout(10)
        assert len(routes) == count, (
            f"Expected {count} requests, received {len(routes)}"
        )

    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.on(
        "console",
        lambda message: (
            errors.append(message.text)
            if message.type == "error"
            and "503 (Service Unavailable)" not in message.text
            else None
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
    saved = {"manim-draft": SOURCE, "manim-tour-seen": "2"}
    writes = []
    pending_gets = []
    held_writes = []
    behavior = {"delay_get": True, "fail": False, "hold": False}

    def preferences(route):
        if route.request.method == "GET":
            if behavior["delay_get"]:
                pending_gets.append(route)
            else:
                route.fulfill(json=saved)
            return
        values = route.request.post_data_json["values"]
        writes.append(values)
        if behavior["hold"]:
            held_writes.append((route, values))
        elif behavior["fail"]:
            route.fulfill(status=503, json={"detail": "Simulated disk write failure"})
        else:
            saved.update(values)
            route.fulfill(json=saved)

    page.route("**/api/preferences", preferences)
    page.route(
        "**/api/health",
        lambda route: route.fulfill(json={"manim": True, "latex": True}),
    )
    page.route("**/api/renders", lambda route: route.fulfill(json=[]))
    page.goto(URL)
    page.wait_for_function("window.codeEditor")
    # An empty, newly mounted editor must never erase the server's draft.
    page.wait_for_timeout(350)
    assert not writes, writes
    behavior["delay_get"] = False
    wait_for_routes(pending_gets, 1)
    pending_gets.pop().fulfill(json=saved)
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    assert page.evaluate("codeEditor.getSource()") == SOURCE
    assert page.evaluate("preferenceChanges['manim-draft']") is None
    assert saved["manim-draft"] == SOURCE
    print("PASS: delayed startup preferences preserve a backend-only draft", flush=True)
    large_source = SOURCE + "# " + "large draft " * 8000
    page.evaluate("source=>setSource(source)", large_source)
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-draft"] == large_source
    print(
        "PASS: drafts above the Chromium keepalive limit save during normal editing",
        flush=True,
    )

    # One replacement should do one analyze, and an older analyze cannot overwrite it.
    analyzes = []
    held_analyzes = []
    delay_analyze = [False]

    def analyze(route):
        source = route.request.post_data_json["source"]
        analyzes.append(source)
        if delay_analyze[0]:
            held_analyzes.append((route, source))
        else:
            route.continue_()

    page.route("**/api/analyze", analyze)
    page.evaluate("source=>setSource(source)", OTHER)
    page.wait_for_timeout(500)
    assert analyzes == [OTHER], analyzes
    delay_analyze[0] = True
    page.evaluate("source=>{void setSource(source)}", SOURCE)
    page.wait_for_function("analysis === null")
    page.evaluate("source=>{void setSource(source)}", OTHER)
    wait_for_routes(held_analyzes, 2)
    held_analyzes[1][0].fulfill(json={"scenes": ["OtherScene"], "error": None})
    page.wait_for_function("analysis?.scenes[0] === 'OtherScene'")
    held_analyzes[0][0].fulfill(json={"scenes": ["MainScene"], "error": None})
    page.wait_for_timeout(50)
    assert page.evaluate("analysis.scenes") == ["OtherScene"]
    delay_analyze[0] = False
    print(
        "PASS: one validation per replacement and stale analyze protection", flush=True
    )

    # Failed saves retain the draft; concurrent edits wait and are sent after the old write.
    page.evaluate("async()=>{await persistPreferences()}")
    behavior["hold"] = True
    page.evaluate("storage.set('manim-name','older');void persistPreferences()")
    wait_for_routes(held_writes, 1)
    page.evaluate("storage.set('manim-name','newer');void persistPreferences()")
    page.wait_for_timeout(50)
    assert len(held_writes) == 1, "Preferences writes must be serialized"
    route, values = held_writes.pop()
    saved.update(values)
    behavior["hold"] = False
    route.fulfill(json=saved)
    page.wait_for_function("preferenceRequest === null")
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-name"] == "newer"
    behavior["fail"] = True
    page.evaluate("source=>setSource(source)", SOURCE)
    for _ in range(3):
        page.evaluate("async()=>{await persistPreferences()}")
    assert page.evaluate("preferenceChanges['manim-draft']") == SOURCE
    assert page.evaluate("preferenceRetries") == 3
    count = len(writes)
    page.wait_for_timeout(1100)
    assert len(writes) == count, "Failed writes must stop automatic retries"
    page.reload()
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    assert page.evaluate("codeEditor.getSource()") == SOURCE
    behavior["fail"] = False
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-draft"] == SOURCE
    assert page.evaluate("localStorage.getItem(pendingPreferencesKey)") is None
    print(
        "PASS: ordered saves, retained failures, bounded retries and reload recovery",
        flush=True,
    )

    # The server's character limit counts Unicode code points, including astral characters.
    astral_source = SOURCE + "# " + "😀" * 260000
    assert len(astral_source) < 500000
    assert page.evaluate("source=>source.length", astral_source) > 500000
    behavior["fail"] = True
    page.evaluate("source=>setSource(source)", astral_source)
    page.evaluate("async()=>{await persistPreferences()}")
    assert page.evaluate("preferenceChanges['manim-draft']") == astral_source
    page.reload()
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    assert page.evaluate("codeEditor.getSource()") == astral_source
    behavior["fail"] = False
    page.evaluate("async()=>{await persistPreferences()}")
    assert saved["manim-draft"] == astral_source
    page.evaluate("source=>setSource(source)", SOURCE)
    page.evaluate("async()=>{await persistPreferences()}")
    print(
        "PASS: pending astral Unicode drafts use the same character limit as Python",
        flush=True,
    )

    behavior["fail"] = True
    page.evaluate("setOutputSettings({width:1500,height:900,fps:17.5})")
    page.evaluate("async()=>{await persistPreferences()}")
    assert page.evaluate("preferenceChanges['manim-resolution']") == "1500x900"
    assert page.evaluate("preferenceChanges['manim-fps']") == "17.5"
    page.reload()
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    expect(page.locator("#resolution-select")).to_have_value("1500x900")
    expect(page.locator("#fps-select")).to_have_value("17.5")
    behavior["fail"] = False
    page.evaluate("async()=>{await persistPreferences()}")
    assert (saved["manim-resolution"], saved["manim-fps"]) == ("1500x900", "17.5")
    page.evaluate("setOutputSettings(legacyOutputSettings.preview)")
    print(
        "PASS: pending custom resolution and fractional FPS survive failed save/reload",
        flush=True,
    )

    # A user edit made before a delayed preferences read remains authoritative.
    behavior["delay_get"] = True
    page.reload()
    page.wait_for_function("window.codeEditor")
    page.locator(".cm-content").focus()
    page.keyboard.insert_text(OTHER)
    wait_for_routes(pending_gets, 1)
    pending_gets.pop().fulfill(json={**saved, "manim-draft": SOURCE})
    behavior["delay_get"] = False
    page.wait_for_function("preferencesReady")
    assert page.evaluate("codeEditor.getSource()") == OTHER
    print(
        "PASS: typing during startup is not replaced by a delayed saved draft",
        flush=True,
    )

    behavior["delay_get"] = True
    page.reload()
    page.wait_for_function("window.codeEditor")
    page.evaluate("source=>setSource(source)", OTHER)
    wait_for_routes(pending_gets, 1)
    pending_gets.pop().fulfill(json={**saved, "manim-draft": SOURCE})
    behavior["delay_get"] = False
    page.wait_for_function("preferencesReady")
    assert page.evaluate("codeEditor.getSource()") == OTHER
    print(
        "PASS: Paste/Import/example source actions also survive delayed restoration",
        flush=True,
    )

    # Scene updates must retain the focused option and provide a normal Tab exit.
    multiple = SOURCE + "\n" + OTHER.split("\n", 1)[1]
    for theme in ("dark", "light"):
        page.evaluate("theme=>applyTheme(theme)", theme)
        page.evaluate("source=>setSource(source)", multiple)
        page.locator("#scene-trigger").focus()
        page.keyboard.press("ArrowDown")
        page.keyboard.press("End")
        expect(page.locator("#scene-list button").last).to_be_focused()
        page.evaluate("syncSceneControl();updateRenderButton()")
        expect(page.locator("#scene-list button").last).to_be_focused()
        page.keyboard.press("Escape")
        expect(page.locator("#scene-trigger")).to_be_focused()
        page.keyboard.press("ArrowDown")
        page.keyboard.press("Tab")
        expect(page.locator("#scene-list")).to_be_hidden()
        expect(page.locator("#resolution-select")).to_be_focused()
        page.locator("#scene-trigger").click()
        page.evaluate("source=>setSource(source)", SOURCE)
        expect(page.locator("#scene-list")).to_be_hidden()
        expect(page.locator("#animation-name")).to_be_focused()
    print(
        "PASS: scene focus survives refresh, Escape and Tab in both themes", flush=True
    )

    # Confirm cancellation must preserve both the script and its quality.
    job = {
        "id": "state-test",
        "status": "completed",
        "name": "Stored",
        "scene": "MainScene",
        "quality": "preview",
        "video": "/static/icon.svg",
        "resolution": "854 × 480",
        "fps": 15,
        "size": 100,
        "elapsed": 1,
        "created": "2026-01-01T00:00:00Z",
    }
    script_requests = []
    hold_script = [False]
    crlf_source = SOURCE.replace("\n", "\r\n")

    def script(route):
        if hold_script[0]:
            script_requests.append(route)
        else:
            route.fulfill(body=crlf_source, content_type="text/plain")

    page.route("**/api/renders/state-test/script", script)
    page.evaluate("job=>{showVideo(job);updateRenderButton()}", job)
    page.evaluate("source=>setSource(source)", OTHER)
    page.locator("#resolution-select").select_option("854x480")
    page.locator("#fps-select").select_option("15")
    page.locator("#higher-quality").click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-cancel").click()
    assert page.evaluate("editor.value") == OTHER
    expect(page.locator("#resolution-select")).to_have_value("854x480")
    expect(page.locator("#fps-select")).to_have_value("15")
    hold_script[0] = True
    page.locator("#higher-quality").click()
    wait_for_routes(script_requests, 1)
    page.evaluate("activeJob={id:'other-render'};updateRenderButton()")
    script_requests.pop().fulfill(body=crlf_source, content_type="text/plain")
    expect(page.locator("#toast")).to_contain_text("workspace changed")
    assert page.evaluate("editor.value") == OTHER
    expect(page.locator("#resolution-select")).to_have_value("854x480")
    expect(page.locator("#fps-select")).to_have_value("15")
    page.evaluate("activeJob=null;updateRenderButton()")
    hold_script[0] = False
    print(
        "PASS: higher-quality cancel and stale script fetch preserve current settings",
        flush=True,
    )

    # Windows render script downloads use CRLF; confirmation and direct reuse both render.
    render_requests = []

    def render(route):
        render_requests.append(route.request.post_data_json)
        route.fulfill(json={**job, "id": "crlf-render", "status": "rendering"})

    page.route("**/api/render", render)
    page.route(
        "**/api/renders/crlf-render",
        lambda route: route.fulfill(
            json={
                **job,
                "id": "crlf-render",
                "quality": "final",
                "width": 1920,
                "height": 1080,
                "fps": 60,
            }
        ),
    )
    page.locator("#higher-quality").click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    with page.expect_request("**/api/render"):
        page.locator("#confirm-replace").click()
    expect(page.locator("#render-button")).to_have_attribute("data-state", "success")
    assert render_requests[-1]["source"] == SOURCE
    assert render_requests[-1]["quality"] == "final"
    assert (
        render_requests[-1]["width"],
        render_requests[-1]["height"],
        render_requests[-1]["fps"],
    ) == (1920, 1080, 60)
    page.evaluate("job=>{showVideo(job);updateRenderButton()}", job)
    with page.expect_request("**/api/render"):
        page.locator("#higher-quality").click()
    expect(page.locator("#render-button")).to_have_attribute("data-state", "success")
    assert len(render_requests) == 2
    assert render_requests[-1]["source"] == SOURCE
    page.evaluate("source=>setSource(source)", OTHER)
    page.evaluate(
        "source=>replaceSource(source,'Imported Windows script')", crlf_source
    )
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-replace").click()
    expect(page.locator(".scene-row")).to_be_hidden()
    expect(page.locator("#scene-select")).to_have_value("MainScene")
    assert page.evaluate("editor.value") == SOURCE
    assert page.evaluate("codeEditor.getSource()") == SOURCE
    page.evaluate("source=>setSource(source)", OTHER)
    print(
        "PASS: CRLF higher-quality confirmation, direct render and imported replacement",
        flush=True,
    )

    # History metadata is text, and cancelled reuse leaves the previous preview selected.
    history_job = {
        **job,
        "resolution": '<img src=x onerror="window.injected=true">',
        "fps": "<svg onload=window.injected=true>",
    }
    page.route("**/api/renders", lambda route: route.fulfill(json=[history_job]))
    page.locator("[data-view=history]").click()
    expect(page.locator(".history-card")).to_be_visible()
    assert page.locator(".history-card img, .history-card p svg").count() == 0
    assert page.evaluate("window.injected") is None
    page.evaluate("lastCompletedJob={id:'previous-preview'}")
    page.locator(".history-card button").click()
    expect(page.locator("#confirm-dialog")).to_be_visible()
    page.locator("#confirm-cancel").click()
    assert page.evaluate("lastCompletedJob.id") == "previous-preview"
    hold_script[0] = True
    page.locator("[data-view=history]").click()
    page.locator(".history-card button").click()
    wait_for_routes(script_requests, 1)
    page.evaluate("source=>setSource(source)", multiple)
    script_requests.pop().fulfill(body=SOURCE, content_type="text/plain")
    expect(page.locator("#toast")).to_contain_text("workspace changed")
    assert page.evaluate("editor.value") == multiple
    expect(page.locator("#confirm-dialog")).to_be_hidden()
    hold_script[0] = False
    page.locator("[data-view=workspace]").click()
    # Assigning a new src resets the media speed unless Studio reapplies the setting.
    page.evaluate(
        "job=>{document.querySelector('#playback-speed').value='1.5';showVideo(job)}",
        job,
    )
    assert page.evaluate("document.querySelector('#video-player').playbackRate") == 1.5
    print(
        "PASS: inert history metadata, cancelled reuse preview, consistent playback speed",
        flush=True,
    )

    # Two history reads can finish in reverse order; only the latest updates the view.
    held_history = []
    page.route("**/api/renders", lambda route: held_history.append(route))
    page.evaluate("showView('history');void loadHistory()")
    wait_for_routes(held_history, 2)
    held_history[1].fulfill(
        json=[{**job, "name": "Newest"}, {**job, "id": "second", "name": "Second"}]
    )
    expect(page.locator("#history-count")).to_have_text("2")
    held_history[0].fulfill(json=[{**job, "name": "Outdated"}])
    page.wait_for_timeout(50)
    expect(page.locator("#history-count")).to_have_text("2")
    expect(page.locator(".history-card h2").first).to_have_text("Newest")
    print(
        "PASS: older history responses cannot replace a newer list or count", flush=True
    )
    page.unroute("**/api/renders")
    page.route("**/api/renders", lambda route: route.fulfill(json=[job]))

    # Startup health/history must not overwrite a render begun while health was pending.
    held_health = []
    page.route("**/api/health", lambda route: held_health.append(route))
    page.route(
        "**/api/renders/startup-current",
        lambda route: route.fulfill(
            json={
                **job,
                "id": "startup-current",
                "status": "rendering",
                "stage": "Rendering",
                "log": "Current",
            }
        ),
    )
    stale_startup_requests = []

    def stale_startup(route):
        stale_startup_requests.append(route)
        route.fulfill(json=job)

    page.route("**/api/renders/startup-old", stale_startup)
    page.reload()
    page.wait_for_function("preferencesReady && analysis?.scenes.length")
    page.evaluate("beginTracking({id:'startup-current',status:'rendering'})")
    wait_for_routes(held_health, 1)
    held_health.pop().fulfill(
        json={"manim": True, "latex": True, "active": "startup-old"}
    )
    expect(page.locator("#history-count")).to_have_text("1")
    assert page.evaluate("activeJob.id") == "startup-current"
    assert page.evaluate("window.lastCompletedJob") is None
    expect(page.locator("#video-actions")).to_be_hidden()
    assert not stale_startup_requests
    page.evaluate("activeJob=null;clearTimeout(pollTimer)")
    print(
        "PASS: delayed startup restoration cannot supersede a current render",
        flush=True,
    )

    held_old_poll = []
    page.route("**/api/renders/poll-old", lambda route: held_old_poll.append(route))
    page.route(
        "**/api/renders/poll-new",
        lambda route: route.fulfill(
            json={
                **job,
                "id": "poll-new",
                "status": "rendering",
                "stage": "New render",
                "log": "New",
            }
        ),
    )
    page.evaluate("beginTracking({id:'poll-old',status:'rendering'})")
    wait_for_routes(held_old_poll, 1)
    page.evaluate("beginTracking({id:'poll-new',status:'rendering'})")
    expect(page.locator("#render-log")).to_have_text("New")
    held_old_poll.pop().fulfill(json=job)
    page.wait_for_timeout(50)
    assert page.evaluate("activeJob.id") == "poll-new"
    expect(page.locator("#render-overlay")).to_be_visible()
    page.evaluate("activeJob=null;clearTimeout(pollTimer)")
    print(
        "PASS: an old completed poll cannot terminate newer render tracking", flush=True
    )

    # The primary action cancels once and remains disabled until cancellation ends.
    held_cancels = []
    cancel_state = {"status": "rendering"}
    page.route(
        "**/api/renders/button-cancel",
        lambda route: route.fulfill(
            json={**job, **cancel_state, "id": "button-cancel", "log": "Rendering"}
        ),
    )
    page.route(
        "**/api/renders/button-cancel/cancel", lambda route: held_cancels.append(route)
    )
    page.evaluate("beginTracking({id:'button-cancel',status:'rendering'})")
    expect(page.locator("#render-button span")).to_have_text("Cancel")
    expect(page.locator("#render-button")).to_be_enabled()
    page.evaluate("invalidateAnalysis()")
    expect(page.locator("#render-button")).to_be_enabled()
    expect(page.locator("#render-button kbd")).to_be_hidden()
    expect(page.locator("#bar-progress-state")).to_have_count(0)
    page.locator("#render-button").click()
    wait_for_routes(held_cancels, 1)
    expect(page.locator("#render-button span")).to_have_text("Cancelling…")
    expect(page.locator("#render-button")).to_be_disabled()
    expect(page.locator("#cancel-button")).to_be_disabled()
    held_cancels.pop().fulfill(json={"status": "cancelling"})
    cancel_state["status"] = "cancelled"
    expect(page.locator("#render-overlay")).to_be_hidden()

    # Terminal cleanup is still busy, but cancellation is no longer available.
    page.route(
        "**/api/renders/cleanup-test",
        lambda route: route.fulfill(
            json={
                **job,
                "id": "cleanup-test",
                "status": "rendering",
                "stage": "Finishing render cleanup…",
                "can_cancel": False,
                "log": "Finishing",
            }
        ),
    )
    page.evaluate("beginTracking({id:'cleanup-test',status:'rendering'})")
    expect(page.locator("#cancel-button")).to_have_text("Finishing render…")
    expect(page.locator("#cancel-button")).to_be_disabled()
    expect(page.locator("#render-button span")).to_have_text("Finishing…")
    expect(page.locator("#render-button")).to_be_disabled()
    page.evaluate("activeJob=null;clearTimeout(pollTimer)")
    assert not errors, errors
    print(
        "PASS: finishing cleanup remains busy with both cancel controls disabled",
        flush=True,
    )
    browser.close()
