"""Windows file selection independent of browser automation's picker interception."""

from __future__ import annotations

import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import threading

AVAILABLE = os.name == "nt"
LOCK = threading.Lock()


class PickerError(RuntimeError):
    pass


class OpenFileName(ctypes.Structure):
    _fields_ = [
        ("lStructSize", wintypes.DWORD),
        ("hwndOwner", wintypes.HWND),
        ("hInstance", wintypes.HINSTANCE),
        ("lpstrFilter", wintypes.LPCWSTR),
        ("lpstrCustomFilter", wintypes.LPWSTR),
        ("nMaxCustFilter", wintypes.DWORD),
        ("nFilterIndex", wintypes.DWORD),
        ("lpstrFile", wintypes.LPWSTR),
        ("nMaxFile", wintypes.DWORD),
        ("lpstrFileTitle", wintypes.LPWSTR),
        ("nMaxFileTitle", wintypes.DWORD),
        ("lpstrInitialDir", wintypes.LPCWSTR),
        ("lpstrTitle", wintypes.LPCWSTR),
        ("Flags", wintypes.DWORD),
        ("nFileOffset", wintypes.WORD),
        ("nFileExtension", wintypes.WORD),
        ("lpstrDefExt", wintypes.LPCWSTR),
        ("lCustData", ctypes.c_ssize_t),
        ("lpfnHook", ctypes.c_void_p),
        ("lpTemplateName", wintypes.LPCWSTR),
        ("pvReserved", ctypes.c_void_p),
        ("dwReserved", wintypes.DWORD),
        ("FlagsEx", wintypes.DWORD),
    ]


def selected_paths(buffer: str) -> list[Path]:
    parts = buffer.split("\0")
    parts = parts[: parts.index("")] if "" in parts else parts
    if not parts:
        return []
    if len(parts) == 1:
        return [Path(parts[0])]
    return [Path(parts[0]) / name for name in parts[1:]]


def choose(kind: str, *, owner: int | None = None) -> list[Path]:
    if not AVAILABLE:
        raise PickerError("The native file picker is available on Windows.")
    if not LOCK.acquire(blocking=False):
        raise PickerError(
            "A file picker is already open. Close it before choosing more files."
        )
    initialized = False
    try:
        library = ctypes.WinDLL("comdlg32", use_last_error=True)
        library.GetOpenFileNameW.argtypes = [ctypes.POINTER(OpenFileName)]
        library.GetOpenFileNameW.restype = wintypes.BOOL
        library.CommDlgExtendedError.restype = wintypes.DWORD
        ole = ctypes.WinDLL("ole32")
        ole.CoInitializeEx.argtypes = [ctypes.c_void_p, wintypes.DWORD]
        ole.CoInitializeEx.restype = ctypes.c_long
        initialized = ole.CoInitializeEx(None, 2) in (0, 1)
        buffer = ctypes.create_unicode_buffer(65536)
        documents = kind == "documents"
        pattern = (
            "*.pdf;*.docx;*.doc;*.txt;*.md;*.rtf;*.odt;*.csv"
            if documents
            else "*.png;*.jpg;*.jpeg;*.webp;*.gif;*.bmp;*.svg;*.wav;*.mp3;*.m4a;*.ogg;*.flac;*.aac;*.mp4;*.webm;*.mov"
        )
        dialog = OpenFileName()
        dialog.lStructSize = ctypes.sizeof(dialog)
        user = ctypes.WinDLL("user32")
        user.GetForegroundWindow.restype = wintypes.HWND
        dialog.hwndOwner = user.GetForegroundWindow() if owner is None else owner
        dialog.lpstrFilter = (
            "Documents" if documents else "Images, audio and video"
        ) + f"\0{pattern}\0All files\0*.*\0\0"
        dialog.nFilterIndex = 1
        dialog.lpstrFile = ctypes.cast(buffer, wintypes.LPWSTR)
        dialog.nMaxFile = len(buffer)
        dialog.lpstrTitle = (
            "Upload documents" if documents else "Upload images or audio"
        )
        # Explorer, multi-select, existing files/paths, and preserve the server CWD.
        dialog.Flags = 0x80000 | 0x200 | 0x1000 | 0x800 | 0x8 | 0x02000000
        if not library.GetOpenFileNameW(ctypes.byref(dialog)):
            error = library.CommDlgExtendedError()
            if error:
                raise PickerError(
                    f"Couldn't open the file picker (Windows error {error})."
                )
            return []
        return selected_paths(buffer[:])
    finally:
        if initialized:
            ole.CoUninitialize()
        LOCK.release()
