"""Verify both real Windows dialogs from the UI, including selection and cancel."""

import ctypes
from ctypes import wintypes
import hashlib
import os
import threading
import time
import wave

from PIL import Image
from playwright.sync_api import expect
from browser_support import api, browser_fixture, open_page, project
from document_fixtures import DIRECTORY


def check():
    if os.name != "nt":
        print("SKIP: the native file picker requires Windows.")
        return
    user = ctypes.WinDLL("user32", use_last_error=True)
    user.FindWindowW.argtypes = [wintypes.LPCWSTR, wintypes.LPCWSTR]
    user.FindWindowW.restype = wintypes.HWND
    user.GetDlgItem.argtypes = [wintypes.HWND, ctypes.c_int]
    user.GetDlgItem.restype = wintypes.HWND
    user.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user.IsWindowVisible.argtypes = [wintypes.HWND]
    user.PostMessageW.argtypes = [
        wintypes.HWND,
        wintypes.UINT,
        wintypes.WPARAM,
        wintypes.LPARAM,
    ]
    user.SendMessageW.argtypes = [
        wintypes.HWND,
        wintypes.UINT,
        wintypes.WPARAM,
        ctypes.c_void_p,
    ]
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user.EnumChildWindows.argtypes = [wintypes.HWND, callback_type, wintypes.LPARAM]

    with browser_fixture(native_file_picker=True) as (browser, url, folder):
        context, page, errors = open_page(browser, url)
        value = project(page, url, "Native file chooser")
        source = folder / "Chosen files"
        source.mkdir()
        pdf = source / "Reference notes.pdf"
        word = source / "Résumé (100% #1).docx"
        pdf.write_bytes((DIRECTORY / "reference.pdf").read_bytes())
        word.write_bytes((DIRECTORY / "reference.docx").read_bytes())
        picture = source / "image.png"
        Image.new("RGB", (16, 16), "blue").save(picture)
        audio = source / "tone.wav"
        with wave.open(str(audio), "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(8000)
            output.writeframes(b"\0\0" * 800)

        def picker(label, files, *, asset_panel=False):
            seen, finish = threading.Event(), threading.Event()
            failures = []

            def operate():
                handle = None
                try:
                    deadline = time.monotonic() + 15
                    while time.monotonic() < deadline:
                        handle = user.FindWindowW("#32770", label)
                        if handle and user.IsWindowVisible(handle):
                            break
                        time.sleep(0.05)
                    assert handle and user.IsWindowVisible(handle), (
                        f"{label}: no visible Windows dialog"
                    )
                    seen.set()
                    assert finish.wait(10)
                    if files is None:
                        user.PostMessageW(handle, 0x0111, 2, 0)
                        return
                    combo = user.GetDlgItem(handle, 1148)
                    edits = []

                    @callback_type
                    def visit(child, _):
                        name = ctypes.create_unicode_buffer(64)
                        user.GetClassNameW(child, name, len(name))
                        if name.value == "Edit":
                            edits.append(child)
                        return True

                    if combo:
                        user.EnumChildWindows(combo, visit, 0)
                    assert edits, "The Windows filename edit control was not found"
                    text = ctypes.create_unicode_buffer(
                        " ".join(f'"{file.resolve()}"' for file in files)
                    )
                    user.SendMessageW(
                        edits[0], 0x000C, 0, ctypes.cast(text, ctypes.c_void_p)
                    )
                    user.PostMessageW(handle, 0x0111, 1, 0)
                except Exception as error:
                    failures.append(str(error))
                    if handle:
                        user.PostMessageW(handle, 0x0111, 2, 0)
                    seen.set()

            worker = threading.Thread(target=operate, daemon=True)
            worker.start()
            if asset_panel:
                page.get_by_role("button", name="Add files", exact=True).click()
            else:
                page.get_by_role("button", name="Attach files", exact=True).click()
                page.get_by_role("menuitem", name=label, exact=True).click()
            assert seen.wait(15), "Windows chooser timed out"
            assert not failures, failures
            if not asset_panel:
                expect(
                    page.get_by_role("button", name="Send", exact=True)
                ).to_be_disabled()
                page.get_by_role("textbox", name="Message the agent").press("Enter")
                assert api(page, url, f"/api/projects/{value['id']}/threads") == {
                    "threads": []
                }
            finish.set()
            worker.join(10)
            assert not failures, failures
            if asset_panel:
                expect(
                    page.get_by_role("button", name="Add files", exact=True)
                ).to_be_enabled()
                if files:
                    expect(
                        page.get_by_text("Reference notes (2).pdf", exact=True)
                    ).to_be_visible()
            elif files is None:
                expect(page.locator("[data-composer] [role=status]")).to_have_count(0)
            else:
                for file in files:
                    expect(
                        page.get_by_role(
                            "button", name=f"Remove {file.name}", exact=True
                        )
                    ).to_be_visible(timeout=15000)
            print(
                f"PASS: visible {label} dialog, {'cancel' if files is None else 'multi-file selection'}",
                flush=True,
            )

        page.get_by_role("textbox", name="Message the agent").fill(
            "Use the selected files"
        )
        picker("Upload documents", [pdf, word])
        picker("Upload images or audio", [picture, audio])
        picker("Upload documents", None)
        picker("Upload images or audio", None)
        page.get_by_role("tab", name="Assets", exact=False).click()
        picker("Upload documents", [pdf], asset_panel=True)
        picker("Upload documents", None, asset_panel=True)
        for file in (pdf, word, picture, audio):
            import urllib.parse

            result = page.request.get(
                url
                + f"/api/projects/{value['id']}/assets/"
                + urllib.parse.quote(file.name, safe="")
            )
            assert (
                hashlib.sha256(result.body()).digest()
                == hashlib.sha256(file.read_bytes()).digest()
            )
        assert not errors, errors
        print("PASS: native picker upload bytes and cancellation", flush=True)
        context.close()


if __name__ == "__main__":
    check()
