"""Run using bundled Python and fresh MANIM_STUDIO_DATA; verify native React UI."""

import ctypes
import json
import sys
import threading
import time
import traceback
import desktop
import webview

original_start = webview.start
failures = []
SOURCE = "from manim import *\nclass NativeDemo(Scene):\n    def construct(self):\n        self.add(Circle(color=BLUE))\n        self.wait(.3)\n"


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

    def asynchronous(expression):
        finished = threading.Event()
        result = []
        window.evaluate_js(
            expression, lambda value: (result.append(value), finished.set())
        )
        assert finished.wait(20), expression
        return result[0]

    try:
        assert window.events.shown.wait(40), "Native window did not appear"
        from System.Windows.Forms import Screen

        form = window.native
        work = Screen.FromHandle(form.Handle).WorkingArea
        assert work.Contains(form.Bounds), (str(work), str(form.Bounds))
        assert abs((form.Left - work.Left) * 2 + form.Width - work.Width) <= 1
        assert abs((form.Top - work.Top) * 2 + form.Height - work.Height) <= 1
        print("PASS: native window opens centered above the taskbar", flush=True)
        wait(
            "Array.from(document.querySelectorAll('button')).some(b=>b.textContent.trim()==='Set up later')"
        )
        window.evaluate_js(
            "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='Set up later').click()"
        )
        wait("!document.querySelector('[role=dialog]')")
        print("PASS: first-run onboarding opens and can be deferred", flush=True)
        pid = asynchronous(
            "fetch('/api/projects', {method:'POST',headers:{'Content-Type':'application/json','X-Studio-Token':document.querySelector('meta[name=studio-token]').content},body:JSON.stringify({name:'Native verification'})}).then(r=>{if(!r.ok)throw new Error('Project creation failed');return r.json()}).then(p=>p.id)"
        )
        assert pid, "No project ID returned"
        window.evaluate_js(f"location.hash='/projects/{pid}'")
        wait("document.querySelector('#tab-script')")
        script = json.dumps(SOURCE)
        assert asynchronous(
            f"fetch('/api/projects/{pid}', {{method:'POST',headers:{{'Content-Type':'application/json','X-Studio-Token':document.querySelector('meta[name=studio-token]').content}},body:JSON.stringify({{source:{script}}})}}).then(r=>r.ok)"
        )
        window.evaluate_js("location.reload()")
        wait("document.querySelector('#tab-script')")
        window.evaluate_js("document.querySelector('#tab-script').click()")
        wait(
            "document.querySelector('.cm-content[contenteditable=true]')?.innerText.includes('NativeDemo')"
        )
        assert (
            window.evaluate_js(
                "getComputedStyle(document.querySelector('.cm-editor')).fontSize"
            )
            == "13px"
        )
        assert window.evaluate_js(
            "document.documentElement.scrollHeight <= innerHeight && document.documentElement.scrollWidth <= innerWidth"
        )
        print("PASS: React, CodeMirror and layout in native WebView2", flush=True)
        assert asynchronous(
            "fetch('/api/health').then(r=>r.json()).then(h=>h.native_file_picker)"
        )
        window.evaluate_js("document.querySelector('#tab-assets').click()")
        wait(
            "Array.from(document.querySelectorAll('button')).some(b=>b.textContent.trim()==='Add files'&&!b.disabled)"
        )
        cancelled = threading.Event()
        picker_errors = []

        def cancel_picker():
            user = ctypes.WinDLL("user32")
            user.FindWindowW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]
            user.FindWindowW.restype = ctypes.c_void_p
            user.IsWindowVisible.argtypes = [ctypes.c_void_p]
            user.PostMessageW.argtypes = [
                ctypes.c_void_p,
                ctypes.c_uint,
                ctypes.c_size_t,
                ctypes.c_ssize_t,
            ]
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                handle = user.FindWindowW("#32770", "Upload documents")
                if handle and user.IsWindowVisible(handle):
                    user.PostMessageW(handle, 0x0111, 2, 0)
                    cancelled.set()
                    return
                time.sleep(0.05)
            picker_errors.append("No visible native file dialog in WebView2")
            cancelled.set()

        threading.Thread(target=cancel_picker, daemon=True).start()
        window.evaluate_js(
            "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='Add files').click()"
        )
        assert cancelled.wait(25) and not picker_errors, picker_errors
        wait(
            "Array.from(document.querySelectorAll('button')).some(b=>b.textContent.trim()==='Add files'&&!b.disabled)"
        )
        print(
            "PASS: visible native upload dialog and cancellation in packaged WebView2",
            flush=True,
        )
        window.evaluate_js("document.querySelector('#tab-preview').click()")
        wait(
            "Array.from(document.querySelectorAll('button')).some(b=>b.textContent.trim()==='Render'&&!b.disabled)"
        )
        window.evaluate_js(
            "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='Render'&&!b.disabled).click()"
        )
        wait("document.querySelector('video')?.readyState>=2", timeout=90)
        assert window.evaluate_js("document.querySelector('video').error") is None
        print("PASS: bundled Manim render and native MP4 playback", flush=True)
        assert asynchronous(
            "fetch('/api/preferences',{method:'POST',headers:{'Content-Type':'application/json','X-Studio-Token':document.querySelector('meta[name=studio-token]').content},body:JSON.stringify({values:{'manim-theme':'light'}})}).then(r=>r.ok)"
        )
        window.evaluate_js("location.reload()")
        wait(
            "document.querySelector('#tab-preview') && !document.documentElement.classList.contains('dark')"
        )
        window.evaluate_js("location.hash='/renders'")
        wait(
            "document.body.innerText.includes('NativeDemo') && document.body.innerText.includes('View video')"
        )
        print("PASS: theme, project and render history persist", flush=True)
        window.evaluate_js("location.hash='/settings/providers'")
        wait("document.body.innerText.includes('CLI binary override')")
        print("PASS: packaged CLI management loads", flush=True)
        print("Browser:", window.evaluate_js("navigator.userAgent"), flush=True)
    except Exception as error:
        traceback.print_exc()
        failures.append(str(error))
    finally:
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
