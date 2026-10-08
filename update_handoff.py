"""Start the independent Windows updater before releasing Studio's file locks."""

import ctypes
from ctypes import wintypes
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

from runtime import DATA_ROOT, PACKAGE_ROOT, PORTABLE, STANDALONE


def cleanup_preflight(directory: Path, root: Path):
    marker = directory / "stage.owner"
    if not marker.is_file() or (root / "update.pending").exists():
        return
    record = marker.read_text(encoding="utf-8-sig").splitlines()
    stage = root.parent / (".manim-update-" + directory.name.removeprefix("install-"))
    if len(record) != 4 or record[:2] != [str(root), str(stage)]:
        return
    if (
        stage.is_symlink()
        or stage.is_junction()
        or stage.resolve().parent != root.parent
    ):
        raise RuntimeError(
            "The update staging directory changed. It was preserved for inspection."
        )
    if stage.is_dir():
        for parent, directories, files in os.walk(stage, followlinks=False):
            for name in directories + files:
                item = Path(parent) / name
                if item.is_symlink() or item.is_junction():
                    raise RuntimeError(
                        "Update staging contains a link and was preserved for inspection."
                    )
        shutil.rmtree(stage)
    marker.unlink(missing_ok=True)


def process_started_ticks() -> int:
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetCurrentProcess.restype = wintypes.HANDLE
    kernel.GetProcessTimes.argtypes = [wintypes.HANDLE] + [
        ctypes.POINTER(wintypes.FILETIME)
    ] * 4
    kernel.GetProcessTimes.restype = wintypes.BOOL
    stamps = [wintypes.FILETIME() for _ in range(4)]
    if not kernel.GetProcessTimes(
        kernel.GetCurrentProcess(), *(ctypes.byref(value) for value in stamps)
    ):
        raise ctypes.WinError(ctypes.get_last_error())
    # Windows FILETIME starts in 1601; .NET DateTime ticks start in year 1.
    return (
        stamps[0].dwHighDateTime << 32 | stamps[0].dwLowDateTime
    ) + 504911232000000000


def launch_update(handoff: dict) -> subprocess.Popen:
    if os.name != "nt" or not STANDALONE:
        raise RuntimeError("Install updates from the packaged Windows app.")
    root = PACKAGE_ROOT.resolve()
    data = DATA_ROOT.resolve()
    if data.is_relative_to(root) and not data.is_relative_to(root / "UserData"):
        raise RuntimeError("Move user data outside the app files before updating.")
    helper = root / "app" / "Studio Update.exe"
    if not helper.is_file():
        raise RuntimeError("The updater is missing. Install the release from GitHub.")
    # The portable launcher can locate this helper even after an interrupted swap.
    cache_root = root / "UserData" if PORTABLE else data
    directory = cache_root / ".studio" / "updates" / ("install-" + uuid.uuid4().hex)
    directory.mkdir(parents=True)
    if not directory.resolve().is_relative_to(cache_root.resolve()):
        raise RuntimeError("The update cache must stay inside your user data folder.")
    # A running helper inside app/ would prevent the portable directory swap.
    copied = directory / helper.name
    shutil.copy2(helper, copied)
    launcher_pid = os.environ.get("MANIM_STUDIO_LAUNCHER_PID", "0")
    launcher_started = os.environ.get("MANIM_STUDIO_LAUNCHER_STARTED", "0")
    if not launcher_pid.isdecimal() or not launcher_started.isdecimal():
        raise RuntimeError("Studio's launcher identity is invalid. Restart the app.")
    # The native helper checks the PID AND process creation time to avoid PID reuse.
    process = subprocess.Popen(
        [
            str(copied),
            "portable" if PORTABLE else "installed",
            str(root),
            str(Path(handoff["path"]).resolve()),
            handoff["sha256"],
            handoff["version"],
            str(os.getpid()),
            str(process_started_ticks()),
            launcher_pid,
            launcher_started,
        ],
        cwd=directory,
        creationflags=subprocess.CREATE_NO_WINDOW,
        close_fds=True,
    )
    deadline = time.monotonic() + 120
    while time.monotonic() < deadline:
        if (directory / "update.ready").is_file():
            return process
        if (directory / "update.log").is_file():
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            cleanup_preflight(directory, root)
            raise RuntimeError(
                "The updater rejected the package. Download it again or use the GitHub release."
            )
        if process.poll() is not None:
            raise RuntimeError(
                "The updater could not prepare the download. Try downloading it again."
            )
        time.sleep(0.05)
    # The helper cannot modify files while this process is alive: it is still
    # hashing or waiting for our exit. Keep Studio open when preparation fails.
    process.terminate()
    process.wait(timeout=5)
    cleanup_preflight(directory, root)
    raise RuntimeError("The updater took too long to prepare. Try again.")
