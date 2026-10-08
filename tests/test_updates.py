"""Adversarial updater tests: official URLs, verified bytes and async transitions."""

import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request

import updates


class Response(io.BytesIO):
    def __init__(self, payload, url, *, header=None, before_read=None):
        super().__init__(payload)
        self.url = url
        self.headers = {
            "Content-Length": str(len(payload)) if header is None else header
        }
        self.before_read = before_read

    def geturl(self):
        return self.url

    def read1(self, size=-1):
        if self.before_read:
            self.before_read()
        return super().read(size)


class Network:
    def __init__(
        self, edition="installed", version="1.1.0", payload=b"verified package"
    ):
        suffix = (
            "Portable-x64.zip" if edition == "portable" else "Offline-Setup-x64.exe"
        )
        self.name = f"Manim-Studio-{version}-{suffix}"
        prefix = updates.REPOSITORY + "/releases/download/v" + version + "/"
        self.asset_url = prefix + self.name
        self.checksum_url = prefix + "SHA256SUMS.txt"
        self.payload = payload
        self.checksums = (
            hashlib.sha256(payload).hexdigest() + "  " + self.name + "\n"
        ).encode()
        self.release = {
            "tag_name": "v" + version,
            "draft": False,
            "prerelease": False,
            "html_url": updates.REPOSITORY + "/releases/tag/v" + version,
            "assets": [
                {
                    "name": self.name,
                    "size": len(payload),
                    "browser_download_url": self.asset_url,
                },
                {
                    "name": "SHA256SUMS.txt",
                    "size": len(self.checksums),
                    "browser_download_url": self.checksum_url,
                },
            ],
        }
        self.calls = []
        self.responses = []
        self.download_payload = payload
        self.header = None
        self.before_read = None
        self.response_url = None

    def open(self, url):
        self.calls.append(url)
        if url == updates.LATEST_API:
            payload = json.dumps(self.release).encode()
            hook = None
        elif url == self.checksum_url:
            payload = self.checksums
            hook = None
        elif url == self.asset_url:
            payload = self.download_payload
            hook = self.before_read
        else:
            raise AssertionError("Unexpected network URL " + url)
        response = Response(
            payload,
            self.response_url or url,
            header=self.header if url == self.asset_url else None,
            before_read=hook,
        )
        self.responses.append(response)
        return response


class UpdateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        log_patch = patch.object(updates.logging, "exception")
        self.log = log_patch.start()
        self.addCleanup(log_patch.stop)
        self.managers = []
        self.addCleanup(self.stop_managers)

    def stop_managers(self):
        for manager in self.managers:
            manager.shutdown()

    def manager(self, edition="installed", **kwargs):
        manager = updates.UpdateManager(
            "1.0.0",
            edition,
            self.root / "package",
            self.root / "data",
            install_supported=kwargs.pop("supported", True),
            **kwargs,
        )
        self.managers.append(manager)
        return manager

    def wait(self, manager, expected=None):
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline:
            snapshot = manager.snapshot()
            if snapshot["status"] not in {"checking", "downloading"}:
                if expected is not None:
                    self.assertEqual(snapshot["status"], expected, snapshot)
                return snapshot
            time.sleep(0.005)
        self.fail("Update worker did not finish")

    def available(self, manager, network):
        manager.check()
        state = self.wait(manager, "available")
        self.assertEqual(state["latest_version"], network.release["tag_name"][1:])

    def test_stable_versions_are_strict_and_numeric(self):
        self.assertGreater(
            updates.stable_version("1.10.0"), updates.stable_version("1.9.0")
        )
        for value in (
            "v1.0.0",
            "01.0.0",
            "1.0",
            "1.0.0-beta",
            "1.0.0+build",
            "1.0.0\n",
            None,
            True,
        ):
            with self.subTest(value=value), self.assertRaises(updates.UpdateError):
                updates.stable_version(value)

    def test_current_older_draft_and_prerelease_are_not_updates(self):
        for version, draft, prerelease in (
            ("1.0.0", False, False),
            ("0.9.0", False, False),
            ("9.0.0", True, False),
            ("9.0.0", False, True),
        ):
            with self.subTest(version=version, draft=draft, prerelease=prerelease):
                network = Network(version=version)
                network.release.update(draft=draft, prerelease=prerelease)
                manager = self.manager()
                with patch.object(updates, "open_github", network.open):
                    manager.check()
                    state = self.wait(manager, "current")
                self.assertIsNone(state["latest_version"])
                self.assertEqual(network.calls, [updates.LATEST_API])

    def test_latest_endpoint_does_not_scan_historical_higher_version(self):
        network = Network(version="1.0.0")
        manager = self.manager()
        with patch.object(updates, "open_github", network.open):
            manager.check()
            self.wait(manager, "current")
        self.assertEqual(network.calls, [updates.LATEST_API])

    def test_404_means_no_public_release(self):
        manager = self.manager()
        with patch.object(
            updates,
            "open_github",
            side_effect=urllib.error.HTTPError(
                updates.LATEST_API, 404, "Not Found", {}, None
            ),
        ):
            manager.check()
            self.wait(manager, "current")

    def test_malformed_successful_release_responses_are_errors_not_current(self):
        for payload in (b"null", b"{}", b"[]", b"false", b"{broken"):
            with self.subTest(payload=payload):
                manager = self.manager()
                with patch.object(
                    updates,
                    "open_github",
                    return_value=Response(payload, updates.LATEST_API),
                ):
                    manager.check()
                    self.wait(manager, "error")

    def test_installed_and_portable_select_exact_matching_asset(self):
        for edition in ("installed", "portable"):
            with self.subTest(edition=edition):
                network = Network(edition)
                manager = self.manager(edition)
                with patch.object(updates, "open_github", network.open):
                    self.available(manager, network)
                    manager.download()
                    state = self.wait(manager, "ready")
                self.assertEqual(state["asset_name"], network.name)
                self.assertEqual(state["bytes_downloaded"], len(network.payload))
                self.assertEqual(state["progress"], 100)
                self.assertNotIn("path", state)
                handoff = manager.begin_install()
                self.assertEqual(handoff["path"].read_bytes(), network.payload)
                self.assertEqual(
                    handoff["sha256"], hashlib.sha256(network.payload).hexdigest()
                )
                self.assertEqual(manager.snapshot()["status"], "installing")
                with self.assertRaises(updates.UpdateBusy):
                    manager.check()
                manager.install_failed("Native handoff failed")
                self.assertEqual(manager.snapshot()["status"], "ready")
                self.assertTrue(all(response.closed for response in network.responses))

    def test_browser_and_source_can_check_but_cannot_download_or_install(self):
        for edition in ("installed", "portable", "source"):
            with self.subTest(edition=edition):
                manager = self.manager(edition, supported=False)
                network = Network(edition)
                with patch.object(updates, "open_github", network.open):
                    self.available(manager, network)
                    with self.assertRaises(updates.UpdateError):
                        manager.download()
                state = manager.snapshot()
                self.assertFalse(state["can_download"])
                self.assertFalse(state["can_install"])
                with self.assertRaises(updates.UpdateError):
                    manager.begin_install()
                manager.set_install_supported(True)
                self.assertEqual(
                    manager.snapshot()["can_download"], edition != "source"
                )

    def test_check_cache_and_error_retry(self):
        manager = self.manager()
        network = Network()
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            manager.check()
            self.assertEqual(len(network.calls), 1)
            with patch.object(updates, "CHECK_INTERVAL", -1):
                manager.check()
                self.wait(manager, "available")
            self.assertEqual(len(network.calls), 2)
            network.download_payload = b"broken package!"
            manager.download()
            self.wait(manager, "error")
            with self.assertRaises(updates.UpdateError):
                manager.download()
            manager.check()
            self.wait(manager, "available")
            self.assertEqual(network.calls.count(updates.LATEST_API), 3)
            network.download_payload = network.payload
            manager.download()
            self.wait(manager, "ready")

    def test_release_validation_rejects_untrusted_or_ambiguous_assets(self):
        for corruption in (
            "wrong_repo",
            "unsafe_url",
            "duplicate",
            "missing",
            "bad_size",
            "huge_size",
            "flags",
            "bad_tag",
        ):
            with self.subTest(corruption=corruption):
                manager = self.manager()
                network = Network()
                if corruption == "wrong_repo":
                    network.release["html_url"] = (
                        "https://github.com/other/repository/releases/tag/v1.1.0"
                    )
                elif corruption == "unsafe_url":
                    network.release["assets"][0]["browser_download_url"] += (
                        "?redirect=elsewhere"
                    )
                elif corruption == "duplicate":
                    network.release["assets"].append(network.release["assets"][0])
                elif corruption == "missing":
                    network.release["assets"].pop(0)
                elif corruption == "bad_size":
                    network.release["assets"][0]["size"] = True
                elif corruption == "huge_size":
                    network.release["assets"][0]["size"] = updates.MAX_ASSET + 1
                elif corruption == "flags":
                    network.release["draft"] = 0
                else:
                    network.release["tag_name"] = "v01.1.0"
                with patch.object(updates, "open_github", network.open):
                    manager.check()
                    self.wait(manager, "error")
                self.assertIsNone(manager.snapshot()["latest_version"])

    def test_unsafe_initial_and_redirect_addresses_are_rejected(self):
        for url in (
            "http://github.com/file",
            "file:///tmp/installer.exe",
            "https://evil.example/file",
            "https://github.com.evil.example/file",
            "https://user@github.com/file",
            "https://github.com:8443/file",
            "https://github.com/file#fragment",
            "https://github.com/\nfile",
        ):
            with self.subTest(url=url):
                with self.assertRaises(updates.UpdateError):
                    updates.validate_https(url)
                with self.assertRaises(updates.UpdateError):
                    updates.GitHubRedirectHandler().redirect_request(
                        urllib.request.Request(updates.REPOSITORY),
                        None,
                        302,
                        "Found",
                        {},
                        url,
                    )
        for host in updates.REDIRECT_HOSTS:
            updates.validate_https("https://" + host + "/asset?signature=test")

    def test_network_opener_uses_https_custom_redirects_and_timeout(self):
        with patch.object(urllib.request, "build_opener") as build:
            updates.open_github(updates.LATEST_API)
            self.assertIsInstance(
                build.call_args.args[0], updates.GitHubRedirectHandler
            )
            args = build.return_value.open.call_args
            self.assertEqual(args.args[0].full_url, updates.LATEST_API)
            self.assertEqual(args.kwargs["timeout"], updates.NETWORK_TIMEOUT)

    def test_oversized_json_and_unsafe_final_response_rejected(self):
        for unsafe in (False, True):
            with self.subTest(unsafe=unsafe):
                manager = self.manager()
                response = Response(
                    b" " * (updates.MAX_JSON + 1),
                    "https://evil.example/file" if unsafe else updates.LATEST_API,
                )
                with patch.object(updates, "open_github", return_value=response):
                    manager.check()
                    self.wait(manager, "error")
                self.assertTrue(response.closed)

    def test_corrupt_or_missing_checksums_never_publish_ready(self):
        for corruption in (
            "wrong_digest",
            "missing",
            "duplicate",
            "invalid",
            "non_ascii",
            "incomplete",
        ):
            with self.subTest(corruption=corruption):
                manager = self.manager()
                network = Network()
                if corruption == "wrong_digest":
                    network.checksums = ("0" * 64 + "  " + network.name + "\n").encode()
                elif corruption == "missing":
                    network.checksums = network.checksums.replace(
                        network.name.encode(), b"another.exe"
                    )
                elif corruption == "duplicate":
                    network.checksums *= 2
                elif corruption == "invalid":
                    network.checksums = b"not a checksum\n"
                elif corruption == "non_ascii":
                    network.checksums = b"\xff"
                else:
                    network.checksums = network.checksums[:-1]
                if corruption != "incomplete":
                    network.release["assets"][1]["size"] = len(network.checksums)
                with patch.object(updates, "open_github", network.open):
                    self.available(manager, network)
                    manager.download()
                    self.wait(manager, "error")
                with self.assertRaises(updates.UpdateError):
                    manager.begin_install()
                self.assertEqual(list((self.root / "data").rglob("*.partial")), [])
                self.assertEqual(list((self.root / "data").rglob("*.exe")), [])

    def test_partial_oversized_and_wrong_length_downloads_are_removed(self):
        for corruption in ("short", "long", "header", "hash"):
            with self.subTest(corruption=corruption):
                manager = self.manager()
                network = Network()
                if corruption == "short":
                    network.download_payload = network.payload[:-1]
                    network.header = str(len(network.payload))
                elif corruption == "long":
                    network.download_payload += b"extra"
                    network.header = str(len(network.payload))
                elif corruption == "header":
                    network.header = "999"
                else:
                    network.download_payload = b"X" * len(network.payload)
                with patch.object(updates, "open_github", network.open):
                    self.available(manager, network)
                    manager.download()
                    self.wait(manager, "error")
                self.assertEqual(list((self.root / "data").rglob("*.partial")), [])
                self.assertEqual(list((self.root / "data").rglob("*.exe")), [])

    def test_atomic_publish_failure_cleans_partial_and_retry_succeeds(self):
        manager = self.manager()
        network = Network()
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            with patch.object(Path, "replace", side_effect=OSError("disk full")):
                manager.download()
                self.wait(manager, "error")
            self.assertEqual(list((self.root / "data").rglob("*.partial")), [])
            manager.check()
            self.wait(manager, "available")
            manager.download()
            self.wait(manager, "ready")

    def test_cancel_blocks_overlap_until_cleanup_then_retry(self):
        manager = self.manager()
        network = Network(payload=b"X" * 500_000)
        entered, release = threading.Event(), threading.Event()
        network.before_read = lambda: (entered.set(), release.wait(2))
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            manager.download()
            self.assertTrue(entered.wait(2))
            manager.cancel()
            with self.assertRaises(updates.UpdateBusy):
                manager.check()
            with self.assertRaises(updates.UpdateBusy):
                manager.download()
            release.set()
            self.wait(manager, "available")
            self.assertEqual(list((self.root / "data").rglob("*.partial")), [])
            network.before_read = None
            manager.download()
            self.wait(manager, "ready")

    def test_shutdown_cannot_publish_late_ready_or_start_new_operations(self):
        manager = self.manager()
        network = Network()
        entered, release = threading.Event(), threading.Event()
        network.before_read = lambda: (entered.set(), release.wait(2))
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            manager.download()
            self.assertTrue(entered.wait(2))
            started = time.monotonic()
            manager.shutdown(timeout=0.02)
            self.assertLess(time.monotonic() - started, 0.2)
            release.set()
            self.wait(manager, "available")
            with self.assertRaises(updates.UpdateError):
                manager.check()
            with self.assertRaises(updates.UpdateError):
                manager.download()
            with self.assertRaises(updates.UpdateError):
                manager.begin_install()
        self.assertEqual(list((self.root / "data").rglob("*.partial")), [])

    def test_check_cancellation_does_not_poison_cache_or_publish_late_release(self):
        manager = self.manager()
        network = Network()
        entered, release = threading.Event(), threading.Event()

        def blocking(url):
            response = network.open(url)
            response.before_read = lambda: (entered.set(), release.wait(2))
            return response

        with patch.object(updates, "open_github", blocking):
            manager.check()
            self.assertTrue(entered.wait(2))
            manager.cancel()
            release.set()
            self.wait(manager, "idle")
            self.assertIsNone(manager.snapshot()["latest_version"])
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)

    def test_cancel_and_shutdown_win_the_release_publication_boundary(self):
        for shutdown in (False, True):
            with self.subTest(shutdown=shutdown):
                manager = self.manager()
                network = Network()
                entered, release = threading.Event(), threading.Event()
                original_lock = manager._lock

                class PublicationLock:
                    def __enter__(self):
                        if (
                            threading.current_thread().name == "studio-update"
                            and not entered.is_set()
                        ):
                            entered.set()
                            release.wait(2)
                        original_lock.acquire()

                    def __exit__(self, *_):
                        original_lock.release()

                manager._lock = PublicationLock()
                with patch.object(updates, "open_github", network.open):
                    manager.check()
                    self.assertTrue(entered.wait(2))
                    if shutdown:
                        manager.shutdown(timeout=0)
                    else:
                        state = manager.cancel()
                        self.assertFalse(state["can_cancel"])
                    release.set()
                    self.wait(manager, "idle")
                self.assertIsNone(manager.snapshot()["latest_version"])
                self.assertIsNone(manager._last_check)
                if not shutdown:
                    with patch.object(updates, "open_github", network.open):
                        self.available(manager, network)

    def test_thread_start_failure_rolls_back_state(self):
        manager = self.manager()
        with patch.object(
            threading.Thread, "start", side_effect=RuntimeError("no thread")
        ):
            with self.assertRaises(updates.UpdateError):
                manager.check()
        self.assertEqual(manager.snapshot()["status"], "idle")
        with patch.object(updates, "open_github", Network().open):
            manager.check()
            self.wait(manager, "available")

    def test_cache_reuses_verified_download_and_reclaims_prior_package_only(self):
        manager = self.manager()
        network = Network()
        directory = self.root / "data/.studio/updates"
        directory.mkdir(parents=True)
        old = directory / "Manim-Studio-0.9.0-Offline-Setup-x64.exe"
        old.write_bytes(b"obsolete verified package")
        partial = directory / ("a" * 32 + ".partial")
        partial.write_bytes(b"interrupted application download")
        unrelated = directory / "user-document.txt"
        unrelated.write_bytes(b"keep")
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            manager.download()
            self.wait(manager, "ready")
            self.assertFalse(old.exists())
            self.assertFalse(partial.exists())
            self.assertEqual(unrelated.read_bytes(), b"keep")
            with patch.object(updates, "CHECK_INTERVAL", -1):
                manager.check()
                self.wait(manager, "ready")
            self.assertEqual(network.calls.count(network.asset_url), 1)
            self.assertEqual(len(list(directory.glob("*.exe"))), 1)

    def test_cache_link_cannot_write_outside_isolated_data(self):
        manager = self.manager()
        network = Network()
        data = self.root / "data"
        (data / ".studio").mkdir(parents=True)
        outside = self.root / "outside"
        outside.mkdir()
        cache = data / ".studio/updates"
        if os.name == "nt":
            result = subprocess.run(
                ["cmd", "/c", "mklink", "/J", str(cache), str(outside)],
                capture_output=True,
                timeout=5,
            )
            if result.returncode:
                self.skipTest("Could not create an isolated directory junction")
            self.addCleanup(cache.rmdir)
        else:
            cache.symlink_to(outside, target_is_directory=True)
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            manager.download()
            self.wait(manager, "error")
        self.assertEqual(list(outside.iterdir()), [])

    def test_terminated_network_stream_removes_partial_and_can_retry(self):
        manager = self.manager()
        network = Network(payload=b"x" * 500_000)
        reads = 0

        def disconnect():
            nonlocal reads
            reads += 1
            if reads == 2:
                raise OSError("connection disconnected")

        network.before_read = disconnect
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            manager.download()
            state = self.wait(manager, "error")
            self.assertGreater(state["bytes_downloaded"], 0)
            self.assertEqual(list((self.root / "data").rglob("*.partial")), [])
            manager.check()
            self.wait(manager, "available")
            network.before_read = None
            manager.download()
            self.wait(manager, "ready")

    def test_slow_incremental_download_obeys_operation_deadline(self):
        manager = self.manager()
        network = Network(payload=b"x" * 500_000)
        network.before_read = lambda: time.sleep(0.015)
        with patch.object(updates, "open_github", network.open):
            self.available(manager, network)
            with patch.object(updates, "DOWNLOAD_DEADLINE", 0.01):
                manager.download()
                state = self.wait(manager, "error")
        self.assertIn("too long", state["error"])
        self.assertEqual(list((self.root / "data").rglob("*.partial")), [])


if __name__ == "__main__":
    unittest.main()
