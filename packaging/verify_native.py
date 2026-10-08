"""Run this using the bundle's runtime/python.exe, with a clean data directory."""

import sys
import time
import ctypes
import desktop
import webview

original_start = webview.start
failures = []


def verify():
    window = webview.windows[0]

    def wait(expression, timeout=40):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                if window.evaluate_js(expression):
                    return
            except Exception:
                pass
            time.sleep(0.1)
        raise AssertionError(expression)

    try:
        wait("window.engineHealth?.manim")
        wait(
            'window.codeEditor && document.querySelector(".cm-content[contenteditable=true]")'
        )
        assert (
            window.evaluate_js(
                'getComputedStyle(document.querySelector(".cm-editor")).fontSize'
            )
            == "14px"
        )
        assert window.evaluate_js(
            'document.documentElement.scrollHeight <= innerHeight && document.querySelector("#render-button").getBoundingClientRect().bottom <= innerHeight'
        )
        print(
            "PASS: CodeMirror and persistent render bar fit the native viewport",
            flush=True,
        )
        assert window.evaluate_js(
            '!document.querySelector("#tour-popover").hidden && !document.querySelector("dialog[open]")'
        )
        wait(
            'Array.from(document.fonts).some(f => f.family === "Host Grotesk" && f.status === "loaded")'
        )
        assert "Host Grotesk" in window.evaluate_js(
            "getComputedStyle(document.body).fontFamily"
        )
        print(
            "PASS: WebView2, local Host Grotesk and nonmodal first-run tour",
            flush=True,
        )
        print("Browser:", window.evaluate_js("navigator.userAgent"), flush=True)
        for _ in range(7):
            assert window.evaluate_js("""(() => {
                const popup=document.querySelector('#tour-popover').getBoundingClientRect();
                const target=document.querySelector('.tour-focus').getBoundingClientRect();
                const within=popup.left>=0 && popup.top>=0 && popup.right<=innerWidth && popup.bottom<=innerHeight;
                const separate=popup.right<=target.left || popup.left>=target.right || popup.bottom<=target.top || popup.top>=target.bottom;
                return within && separate && document.querySelectorAll('.tour-focus').length===1;
            })()""")
            window.evaluate_js('document.querySelector("#tour-next").click()')
        assert window.evaluate_js(
            'document.querySelector("#tour-popover").hidden && !document.querySelector(".tour-focus")'
        )
        window.evaluate_js('document.querySelector("#help-button").click()')
        window.evaluate_js('document.querySelector("#load-sample").click()')
        wait('!document.querySelector("#render-button").disabled')
        assert window.evaluate_js('!document.querySelector("#tour-popover").hidden')
        window.evaluate_js(
            'document.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true}))'
        )
        assert window.evaluate_js('document.querySelector("#tour-popover").hidden')
        print(
            "PASS: seven anchored steps, target visibility, replay, interaction and Escape",
            flush=True,
        )
        window.evaluate_js('document.querySelector("#render-button").click()')
        wait(
            '!document.querySelector("#video-actions").hidden && document.querySelector("#video-player").readyState >= 2'
        )
        assert window.evaluate_js(
            'document.querySelector("#video-player").error === null'
        )
        print("PASS: bundled Manim render and native MP4 playback", flush=True)
        window.evaluate_js('document.querySelector("#theme-button").click()')
        assert window.evaluate_js("document.documentElement.dataset.theme") == "light"
        window.evaluate_js("location.reload()")
        wait(
            'document.documentElement.dataset.theme === "light" && document.querySelector("#script-input").value.includes("class MainScene")'
        )
        assert window.evaluate_js('document.querySelector("#tour-popover").hidden')
        print("PASS: theme, draft and tour persist in standalone storage", flush=True)
        window.evaluate_js('document.querySelector("[data-view=history]").click()')
        wait('document.querySelectorAll(".history-card").length > 0')
        print("PASS: bundled render history", flush=True)
    except Exception as error:
        failures.append(str(error))
        print("FAIL:", error, flush=True)
    finally:
        # Post Close without blocking the worker on Python closing callbacks.
        handle = int(window.native.Handle.ToInt64())
        ctypes.windll.user32.PostMessageW.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_size_t,
            ctypes.c_ssize_t,
        ]
        ctypes.windll.user32.PostMessageW(handle, 0x0010, 0, 0)


def start(**kwargs):
    return original_start(func=verify, **kwargs)


webview.start = start
sys.argv = ["desktop.py", "--port", "18866"]
desktop.main()
if failures:
    raise SystemExit(1)
