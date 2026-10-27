"""Start Studio in a native window; --browser serves the same UI for development."""

import argparse
import logging
import socket
import threading
import time
import webbrowser
import os

import uvicorn
from events import BUS
from studio import app, shutdown_render, configure_update_install, update_install_failed
import studio
from runtime import DATA_ROOT, PACKAGE_ROOT, STANDALONE
from webview_runtime import browser_mode, ensure_evergreen


def initial_window_bounds(work_area, size, scale=1.0):
    """Fit the outer window inside a monitor's physical working rectangle."""
    left, top, available_width, available_height = work_area
    padding = min(round(24 * scale), (min(available_width, available_height) - 1) // 2)
    width = min(size[0], available_width - 2 * padding)
    height = min(size[1], available_height - 2 * padding)
    return (
        left + (available_width - width) // 2,
        top + (available_height - height) // 2,
        width,
        height,
    )


def position_initial_window(window):
    # before_show runs on the GUI thread after the native form exists. Using its
    # physical bounds avoids applying DPI scaling twice or losing monitor offsets.
    from System.Drawing import Size
    from System.Windows.Forms import FormStartPosition, Screen

    form = window.native
    work = Screen.FromHandle(form.Handle).WorkingArea
    x, y, width, height = initial_window_bounds(
        (work.X, work.Y, work.Width, work.Height),
        (form.Width, form.Height),
        form.DeviceDpi / 96,
    )
    form.MinimumSize = Size(
        min(form.MinimumSize.Width, width), min(form.MinimumSize.Height, height)
    )
    form.StartPosition = FormStartPosition.Manual
    form.SetBounds(x, y, width, height)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser", action="store_true")
    parser.add_argument("--port", type=int, default=0 if STANDALONE else 8765)
    parser.add_argument("--no-open", action="store_true")
    args = parser.parse_args()
    # Only the app itself checks for agent CLI updates on its own (not tests).
    studio.AGENT_UPDATES.automatic = (
        os.environ.get("MANIM_STUDIO_AGENT_UPDATES", "1") != "0"
    )
    with socket.socket() as sock:
        try:
            sock.bind(("127.0.0.1", args.port))
        except OSError:
            raise RuntimeError(
                f"Port {args.port} is in use. Close the other Studio window and try again."
            )
        port = sock.getsockname()[1]
    url = f"http://127.0.0.1:{port}"
    if args.browser:
        if not args.no_open:
            threading.Timer(1.0, lambda: webbrowser.open(url)).start()
        uvicorn.run(
            app,
            host="127.0.0.1",
            port=port,
            log_level="warning",
            log_config=None,
            timeout_graceful_shutdown=3,
        )
        return
    log_dir = DATA_ROOT / "outputs"
    log_dir.mkdir(exist_ok=True)
    logging.basicConfig(filename=log_dir / "studio-startup.log", level=logging.INFO)
    shared_browser = browser_mode(PACKAGE_ROOT) == "evergreen"
    if STANDALONE and os.name == "nt" and shared_browser:
        ensure_evergreen(PACKAGE_ROOT)
    server = uvicorn.Server(
        uvicorn.Config(
            app,
            host="127.0.0.1",
            port=port,
            log_level="warning",
            log_config=None,
            timeout_graceful_shutdown=3,
        )
    )
    thread = threading.Thread(target=server.run, daemon=True)
    thread.start()
    deadline = time.monotonic() + 20
    while not server.started:
        if not thread.is_alive() or time.monotonic() > deadline:
            raise RuntimeError(
                "Studio could not start its local server. See outputs/studio-startup.log."
            )
        time.sleep(0.05)
    try:
        import webview

        webview.settings["ALLOW_DOWNLOADS"] = True
        if shared_browser:
            os.environ.pop("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", None)
            webview.settings["WEBVIEW2_RUNTIME_PATH"] = None
        elif (PACKAGE_ROOT / "webview2" / "msedgewebview2.exe").is_file():
            webview.settings["WEBVIEW2_RUNTIME_PATH"] = str(PACKAGE_ROOT / "webview2")
        window = webview.create_window(
            "Manim Studio",
            url,
            width=1440,
            height=960,
            min_size=(820, 620),
            background_color="#0a0a0a",
            text_select=True,
        )
        if os.name == "nt":
            window.events.before_show += position_initial_window

        def install_update(handoff):
            from update_handoff import launch_update

            process = launch_update(handoff)

            def close_for_update():
                try:
                    window.destroy()
                except Exception:
                    logging.exception("Unable to close Studio for its update")
                    # This helper waits for us, so cancelling it cannot interrupt installation.
                    process.terminate()
                    update_install_failed(
                        "Studio could not close. Close and reopen it, then retry."
                    )

            timer = threading.Timer(1, close_for_update)
            timer.daemon = True
            timer.start()

        if STANDALONE and os.name == "nt":
            configure_update_install(install_update)

        closing = {"allowed": False, "saving": False}

        def prepare_close():
            from studio import ACTIVE, AGENTS

            # Keep the native message loop free while the JavaScript Promise
            # finishes. The closing event itself runs synchronously on the GUI.
            saved = threading.Event()
            result = []
            try:
                window.evaluate_js(
                    "window.studioLifecycle ? window.studioLifecycle.flush() : true",
                    lambda value: (result.append(value), saved.set()),
                )
                if not saved.wait(10) or not result or result[0] is not True:
                    if not window.create_confirmation_dialog(
                        "Changes could not be saved",
                        "Your pending draft is kept for recovery. Close Studio anyway?",
                    ):
                        closing["saving"] = False
                        return
            except Exception:
                logging.exception("Unable to save the workspace before closing")
                if not window.create_confirmation_dialog(
                    "Workspace is still saving",
                    "Studio couldn't finish saving. Close anyway?",
                ):
                    closing["saving"] = False
                    return
            if ACTIVE:
                if not window.create_confirmation_dialog(
                    "Video is still rendering",
                    "Closing Studio will cancel this render. Close anyway?",
                ):
                    closing["saving"] = False
                    return
            if AGENTS.running:
                if not window.create_confirmation_dialog(
                    "An agent is still working",
                    "Closing Studio will stop the agent. Close anyway?",
                ):
                    closing["saving"] = False
                    return
            closing["allowed"] = True
            window.destroy()

        def confirm_close():
            if closing["allowed"]:
                return True
            if not closing["saving"]:
                closing["saving"] = True
                threading.Thread(target=prepare_close, daemon=True).start()
            return False

        window.events.closing += confirm_close
        storage = DATA_ROOT / ".studio" / "webview"
        storage.mkdir(parents=True, exist_ok=True)
        icon = PACKAGE_ROOT / "app" / "studio.ico"
        webview.start(
            gui="edgechromium",
            private_mode=False,
            storage_path=str(storage),
            icon=str(icon) if icon.is_file() else None,
        )
    except Exception:
        logging.exception("Native window unavailable")
        if STANDALONE:
            raise RuntimeError(
                f"Studio could not open its window. Restart the app. If it persists, see {log_dir / 'studio-startup.log'}."
            )
        webbrowser.open(url)
        try:
            while thread.is_alive():
                time.sleep(0.5)
        except KeyboardInterrupt:
            pass
    finally:
        configure_update_install(None)
        try:
            shutdown_render()
        finally:
            BUS.close()
            server.should_exit = True
            thread.join(timeout=5)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        if os.name == "nt":
            import ctypes

            ctypes.windll.user32.MessageBoxW(
                0, str(error), "Manim Studio could not open", 0x10
            )
        raise
