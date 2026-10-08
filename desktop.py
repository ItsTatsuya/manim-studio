"""Start Studio in a native window; --browser serves the same UI for development."""

import argparse
import logging
import socket
import threading
import time
import webbrowser
import os

import uvicorn
from studio import app, shutdown_render, configure_update_install, update_install_failed
from runtime import DATA_ROOT, PACKAGE_ROOT, STANDALONE
from webview_runtime import browser_mode, ensure_evergreen


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--browser", action="store_true")
    parser.add_argument("--port", type=int, default=0 if STANDALONE else 8765)
    parser.add_argument("--no-open", action="store_true")
    args = parser.parse_args()
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
            app, host="127.0.0.1", port=port, log_level="warning", log_config=None
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
            app, host="127.0.0.1", port=port, log_level="warning", log_config=None
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
            background_color="#111113",
            text_select=True,
        )

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

        def confirm_close():
            from studio import ACTIVE

            if ACTIVE:
                return window.create_confirmation_dialog(
                    "Video is still rendering",
                    "Closing Studio will cancel this render. Close anyway?",
                )
            return True

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
