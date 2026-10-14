"""Explicit, verified updates from the official GitHub release; no install execution."""

from __future__ import annotations

import hashlib
import http.client
import json
import logging
from pathlib import Path
import re
import socket
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

REPOSITORY = "https://github.com/ItsTatsuya/manim-studio"
LATEST_API = "https://api.github.com/repos/ItsTatsuya/manim-studio/releases/latest"
RELEASES_URL = REPOSITORY + "/releases/latest"
CHECK_INTERVAL = 15 * 60
MAX_JSON = 2 * 1024 * 1024
MAX_CHECKSUMS = 64 * 1024
MAX_ASSET = 4 * 1024 * 1024 * 1024
NETWORK_TIMEOUT = 5
DOWNLOAD_DEADLINE = 60 * 60
REDIRECT_HOSTS = {
    "github.com",
    "api.github.com",
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
}
VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\Z")


class UpdateError(Exception):
    """An actionable update failure or invalid transition."""


class UpdateBusy(UpdateError):
    """An update operation already owns the updater."""


class _Cancelled(Exception):
    pass


def stable_version(value: str) -> tuple[int, int, int]:
    if not isinstance(value, str) or len(value) > 64 or not VERSION.fullmatch(value):
        raise UpdateError("The release has an unsupported version number.")
    return tuple(int(part) for part in value.split("."))


def validate_https(url: str) -> None:
    try:
        parsed = urllib.parse.urlsplit(url)
        valid = (
            parsed.scheme == "https"
            and parsed.hostname in REDIRECT_HOSTS
            and parsed.port in {None, 443}
            and parsed.username is None
            and parsed.password is None
            and not parsed.fragment
            and not any(ord(char) < 32 or ord(char) == 127 for char in url)
        )
    except (TypeError, ValueError):
        valid = False
    if not valid:
        raise UpdateError(
            "GitHub returned an unsafe download address. Update cancelled."
        )


class GitHubRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        validate_https(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def open_github(url: str):
    validate_https(url)
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": "Manim-Studio-Updater",
            "Accept": "application/vnd.github+json"
            if url == LATEST_API
            else "application/octet-stream",
            "Accept-Encoding": "identity",
        },
    )
    return urllib.request.build_opener(GitHubRedirectHandler()).open(
        request, timeout=NETWORK_TIMEOUT
    )


class UpdateManager:
    def __init__(
        self, current_version, edition, package_root, data_root, install_supported=False
    ):
        self.current_version = current_version
        self._current = stable_version(current_version)
        if edition not in {"installed", "portable", "source"}:
            raise ValueError("Unknown Studio edition")
        self.edition = edition
        self._install_supported = bool(install_supported) and edition != "source"
        self.package_root = Path(package_root).resolve()
        self.data_root = Path(data_root).resolve()
        self._lock = threading.RLock()
        self._thread = None
        self._cancel = threading.Event()
        self._closed = False
        self._last_check = None
        self._release = None
        self._ready = None
        self._state = {
            "status": "idle",
            "progress": 0,
            "bytes_downloaded": 0,
            "total_bytes": 0,
            "error": None,
        }

    def snapshot(self):
        with self._lock:
            status = self._state["status"]
            return {
                **self._state,
                "current_version": self.current_version,
                "edition": self.edition,
                "latest_version": self._release["version"] if self._release else None,
                "release_url": self._release["url"] if self._release else RELEASES_URL,
                "asset_name": self._release["asset"]["name"]
                if self._release and "asset" in self._release
                else None,
                "can_download": self._install_supported,
                "can_install": self._install_supported,
                "can_cancel": not self._closed
                and status in {"checking", "downloading"}
                and not self._cancel.is_set(),
            }

    def set_install_supported(self, enabled):
        with self._lock:
            self._install_supported = bool(enabled) and self.edition != "source"

    def _start(self, status, target):
        with self._lock:
            if self._closed:
                raise UpdateError("Studio is closing. Restart it to check for updates.")
            if self._thread is not None and self._thread.is_alive():
                raise UpdateBusy("An update operation is already running.")
            if self._state["status"] == "installing":
                raise UpdateBusy("The update installer is starting.")
            previous = dict(self._state)
            self._cancel = threading.Event()
            self._state.update(
                status=status, error=None, progress=0, bytes_downloaded=0, total_bytes=0
            )
            thread = threading.Thread(
                target=self._run,
                args=(target, self._cancel),
                name="studio-update",
                daemon=True,
            )
            self._thread = thread
            try:
                thread.start()
            except (RuntimeError, OSError):
                self._thread = None
                self._state = previous
                raise UpdateError(
                    "Studio could not start the update check. Try again."
                ) from None
            return self.snapshot()

    def check(self):
        with self._lock:
            if self._closed:
                raise UpdateError("Studio is closing. Restart it to check for updates.")
            if self._state["status"] in {"checking", "downloading", "installing"}:
                raise UpdateBusy("An update operation is already running.")
            if (
                self._state["status"] != "error"
                and self._last_check is not None
                and time.monotonic() - self._last_check < CHECK_INTERVAL
            ):
                return self.snapshot()
            return self._start("checking", self._check)

    def download(self):
        with self._lock:
            if self._closed:
                raise UpdateError("Studio is closing. Restart it to download updates.")
            if self._state["status"] in {"checking", "downloading", "installing"}:
                raise UpdateBusy("An update operation is already running.")
            if self._state["status"] == "ready":
                return self.snapshot()
            if (
                not self._install_supported
                or self._state["status"] != "available"
                or not self._release
            ):
                raise UpdateError("Check for an available Windows update first.")
            return self._start("downloading", self._download)

    def cancel(self):
        with self._lock:
            if self._state["status"] == "installing":
                raise UpdateBusy("The update installer is starting.")
            self._cancel.set()
            return self.snapshot()

    def shutdown(self, timeout=5):
        with self._lock:
            self._closed = True
            self._cancel.set()
            thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout=max(0, timeout))

    def begin_install(self):
        """Return verified private handoff data; the native host owns all execution."""
        with self._lock:
            if (
                self._closed
                or not self._install_supported
                or self._state["status"] != "ready"
                or self._ready is None
            ):
                raise UpdateError(
                    "Download and verify the update before installing it."
                )
            self._state["status"] = "installing"
            return dict(self._ready)

    def install_failed(self, message):
        with self._lock:
            if self._state["status"] != "installing":
                raise UpdateError("No update installer is starting.")
            self._state.update(status="ready", error=str(message)[:500])
            return self.snapshot()

    def _interrupted(self, cancel, deadline):
        if cancel.is_set():
            raise _Cancelled
        if time.monotonic() > deadline:
            raise UpdateError(
                "The GitHub download took too long. Check your connection and retry."
            )

    def _read(self, url, maximum, cancel, deadline):
        self._interrupted(cancel, deadline)
        result = bytearray()
        with open_github(url) as response:
            validate_https(response.geturl())
            while True:
                self._interrupted(cancel, deadline)
                chunk = response.read1(min(64 * 1024, maximum + 1 - len(result)))
                if not chunk:
                    return bytes(result)
                result.extend(chunk)
                if len(result) > maximum:
                    raise UpdateError(
                        "GitHub returned an unexpectedly large update response."
                    )

    def _check(self, cancel):
        deadline = time.monotonic() + 30
        no_public_release = False
        try:
            payload = self._read(LATEST_API, MAX_JSON, cancel, deadline)
            release = json.loads(payload)
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
            release = None
            no_public_release = True
        if not no_public_release and not isinstance(release, dict):
            raise UpdateError(
                "GitHub returned invalid release information. Try again later."
            )
        selected = None
        if not no_public_release and (
            type(release.get("draft")) is not bool
            or type(release.get("prerelease")) is not bool
        ):
            raise UpdateError(
                "GitHub returned invalid release information. Try again later."
            )
        if (
            release
            and release.get("draft") is False
            and release.get("prerelease") is False
        ):
            tag = release.get("tag_name")
            if not isinstance(tag, str) or not tag.startswith("v"):
                raise UpdateError("The release has an unsupported version number.")
            version = tag[1:]
            if stable_version(version) > self._current:
                url = REPOSITORY + "/releases/tag/" + tag
                if release.get("html_url") != url:
                    raise UpdateError(
                        "GitHub returned a release outside the official repository."
                    )
                selected = {"version": version, "url": url}
                if self.edition != "source":
                    suffix = (
                        "Portable-x64.zip"
                        if self.edition == "portable"
                        else "Offline-Setup-x64.exe"
                    )
                    name = f"Manim-Studio-{version}-{suffix}"
                    selected["asset"] = self._asset(release, tag, name, MAX_ASSET)
                    selected["checksums"] = self._asset(
                        release, tag, "SHA256SUMS.txt", MAX_CHECKSUMS
                    )
        self._interrupted(cancel, deadline)
        with self._lock:
            self._interrupted(cancel, deadline)
            ready = (
                self._ready
                if selected
                and self._release
                and selected["version"] == self._release["version"]
                else None
            )
            self._release = selected
            self._ready = ready
            self._last_check = time.monotonic()
            self._state.update(error=None)
        return "ready" if ready else "available" if selected else "current"

    def _asset(self, release, tag, name, maximum):
        assets = release.get("assets")
        if not isinstance(assets, list):
            raise UpdateError("The release does not include its Windows update files.")
        matches = [
            asset
            for asset in assets
            if isinstance(asset, dict) and asset.get("name") == name
        ]
        if len(matches) != 1:
            raise UpdateError(
                "The release is missing a unique " + name + ". Try again later."
            )
        asset = matches[0]
        url = REPOSITORY + "/releases/download/" + tag + "/" + name
        size = asset.get("size")
        if (
            type(size) is not int
            or not 0 < size <= maximum
            or asset.get("browser_download_url") != url
        ):
            raise UpdateError("The release contains an invalid Windows update file.")
        return {"name": name, "url": url, "size": size}

    def _download(self, cancel):
        with self._lock:
            release = dict(self._release)
            self._state["total_bytes"] = release["asset"]["size"]
        deadline = time.monotonic() + DOWNLOAD_DEADLINE
        asset = release["asset"]
        checksums = self._read(
            release["checksums"]["url"], MAX_CHECKSUMS, cancel, deadline
        )
        if len(checksums) != release["checksums"]["size"]:
            raise UpdateError("The release checksum file was incomplete. Try again.")
        expected = self._checksum(checksums, asset["name"])
        directory = self.data_root / ".studio" / "updates"
        directory.mkdir(parents=True, exist_ok=True)
        if directory.resolve() != self.data_root / ".studio" / "updates":
            raise UpdateError(
                "The update cache points outside Studio data. Choose a safe data folder."
            )
        temporary = directory / (uuid.uuid4().hex + ".partial")
        final = directory / asset["name"]
        try:
            digest = hashlib.sha256()
            received = 0
            with open_github(asset["url"]) as response, temporary.open("xb") as output:
                validate_https(response.geturl())
                length = response.headers.get("Content-Length")
                if length is not None and length != str(asset["size"]):
                    raise UpdateError(
                        "The Windows update download has an unexpected size."
                    )
                while True:
                    self._interrupted(cancel, deadline)
                    # One socket read lets cancellation/deadlines interrupt slow trickles.
                    chunk = response.read1(256 * 1024)
                    if not chunk:
                        break
                    received += len(chunk)
                    if received > asset["size"]:
                        raise UpdateError(
                            "The Windows update download exceeded its expected size."
                        )
                    output.write(chunk)
                    digest.update(chunk)
                    with self._lock:
                        self._state.update(
                            bytes_downloaded=received,
                            total_bytes=asset["size"],
                            progress=round(received / asset["size"] * 100, 1),
                        )
            if received != asset["size"] or digest.hexdigest() != expected:
                raise UpdateError(
                    "The Windows update failed checksum verification. No installer was started. Retry the download."
                )
            self._interrupted(cancel, deadline)
            with self._lock:
                self._interrupted(cancel, deadline)
                temporary.replace(final)
                self._ready = {
                    "path": final,
                    "version": release["version"],
                    "asset": asset["name"],
                    "sha256": expected,
                    "size": asset["size"],
                }
                self._state.update(progress=100, error=None)
            # Keep one verified package; filenames are generated by this updater.
            for old in directory.iterdir():
                if (
                    old != final
                    and re.fullmatch(
                        r"(?:Manim-Studio-[0-9]+\.[0-9]+\.[0-9]+-(?:Offline-Setup-x64\.exe|Portable-x64\.zip)|[0-9a-f]{32}\.partial)",
                        old.name,
                    )
                    and old.resolve().parent == directory
                ):
                    try:
                        old.unlink()
                    except OSError:
                        logging.exception("Could not remove an obsolete update package")
        finally:
            temporary.unlink(missing_ok=True)
        return "ready"

    @staticmethod
    def _checksum(payload, name):
        try:
            text = payload.decode("ascii")
        except UnicodeDecodeError:
            raise UpdateError("The release checksum file is invalid.") from None
        matches = []
        for line in text.splitlines():
            match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *](.+)", line)
            if not match:
                raise UpdateError("The release checksum file is invalid.")
            if match[2] == name:
                matches.append(match[1].lower())
        if len(matches) != 1:
            raise UpdateError(
                "The update has no unique trusted checksum. No installer was started."
            )
        return matches[0]

    def _run(self, target, cancel):
        try:
            status = target(cancel)
            with self._lock:
                self._state["status"] = status
                self._thread = None
        except _Cancelled:
            with self._lock:
                self._state.update(
                    status="available" if self._release else "idle",
                    progress=0,
                    bytes_downloaded=0,
                    total_bytes=0,
                    error=None,
                )
                self._thread = None
        except Exception as error:
            logging.exception("Studio update operation failed")
            if isinstance(error, UpdateError):
                message = str(error)
            elif isinstance(error, urllib.error.HTTPError) and error.code == 403:
                message = "GitHub refused the update check. Its request limit may be reached. Try again later."
            elif isinstance(
                error,
                (
                    OSError,
                    socket.timeout,
                    urllib.error.URLError,
                    http.client.HTTPException,
                ),
            ):
                message = "Studio could not reach or save the GitHub update. Check your connection and available disk space, then retry."
            else:
                message = "GitHub returned invalid update information. Try again later."
            with self._lock:
                self._state.update(status="error", error=message)
                self._thread = None
