"""Use the shared Microsoft WebView2 Runtime, with a verified first-run fallback."""

from __future__ import annotations

import hashlib
import json
import logging
import os
from pathlib import Path
import re
import subprocess

CLIENT = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"


def evergreen_version() -> str | None:
    if os.name != "nt":
        return None
    import winreg

    for hive, key in (
        (winreg.HKEY_CURRENT_USER, rf"Software\Microsoft\EdgeUpdate\Clients\{CLIENT}"),
        (
            winreg.HKEY_LOCAL_MACHINE,
            rf"Software\WOW6432Node\Microsoft\EdgeUpdate\Clients\{CLIENT}",
        ),
    ):
        try:
            with winreg.OpenKey(hive, key) as handle:
                version, _ = winreg.QueryValueEx(handle, "pv")
            if isinstance(version, str) and re.fullmatch(
                r"[0-9]+(?:\.[0-9]+){3}", version
            ):
                if any(int(part) for part in version.split(".")):
                    return version
        except OSError:
            pass
    return None


def browser_mode(package_root: Path) -> str:
    metadata = package_root / "standalone.json"
    if metadata.is_file():
        return json.loads(metadata.read_text(encoding="utf-8")).get("webview2", "fixed")
    return "evergreen"


def ensure_evergreen(package_root: Path) -> str:
    if version := evergreen_version():
        return version
    message = (
        "Microsoft WebView2 Runtime is required. Connect to the internet and reopen "
        "Studio, or run the offline Studio installer to prepare the browser."
    )
    manifest = package_root / "app" / "webview-evergreen.json"
    if not manifest.is_file():
        raise RuntimeError(message)
    record = json.loads(manifest.read_text(encoding="utf-8"))["installers"][
        "bootstrapper"
    ]
    name = record["filename"]
    if Path(name).name != name:
        raise RuntimeError("Invalid WebView2 installer filename")
    installer = package_root / "tools" / name
    with installer.open("rb") as source:
        if hashlib.file_digest(source, "sha256").hexdigest() != record["sha256"]:
            raise RuntimeError(
                "WebView2 installer verification failed. Reinstall Studio."
            )
    logging.info("Preparing shared Microsoft WebView2 Runtime")
    try:
        result = subprocess.run(
            [str(installer), "/silent", "/install"],
            creationflags=subprocess.CREATE_NO_WINDOW,
            timeout=300,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(message) from error
    version = evergreen_version()
    if result.returncode not in (0, 3010) or version is None:
        raise RuntimeError(message)
    return version
