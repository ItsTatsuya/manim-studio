"""Real Windows helper transactions and native/render admission boundaries."""

import hashlib
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import MagicMock, patch
import uuid
from zipfile import ZipFile

from fastapi import HTTPException
import runtime
import studio
from update_handoff import process_started_ticks, cleanup_preflight

ROOT = Path(__file__).resolve().parents[1]
COMPILER = (
    Path(os.environ.get("WINDIR", "C:/Windows"))
    / "Microsoft.NET/Framework64/v4.0.30319/csc.exe"
)


class UpdateIntegrationTests(unittest.TestCase):
    def test_version_metadata_uses_the_same_canonical_validation_as_updater(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            with (
                patch.object(runtime, "PACKAGE_ROOT", path),
                patch.object(runtime, "STANDALONE", True),
            ):
                for version in ("01.0.0", "١.0.0", "1" * 65 + ".0.0", None, [], {}):
                    (path / "standalone.json").write_text(
                        json.dumps({"version": version})
                    )
                    with self.assertLogs(level="WARNING"):
                        self.assertEqual(runtime.app_version(), "1.0.0")
                (path / "standalone.json").write_text(json.dumps({"version": "1.2.3"}))
                self.assertEqual(runtime.app_version(), "1.2.3")
                (path / "standalone.json").write_text("[" * 2000 + "]" * 2000)
                with self.assertLogs(level="WARNING"):
                    self.assertEqual(runtime.app_version(), "1.0.0")

    def test_install_reserves_render_admission_and_launch_failure_releases_it(self):
        entered, release = threading.Event(), threading.Event()
        manager = MagicMock()
        manager.begin_install.return_value = {"path": "verified"}
        manager.snapshot.return_value = {"status": "installing"}

        def prepare(_):
            entered.set()
            self.assertTrue(release.wait(5))

        with (
            patch.object(studio, "UPDATES", manager),
            patch.object(studio, "ACTIVE", None),
            patch.object(studio, "UPDATE_INSTALLING", False),
            patch.object(studio, "UPDATE_HANDLER", prepare),
        ):
            worker = threading.Thread(target=studio.install_update)
            worker.start()
            try:
                self.assertTrue(entered.wait(5))
                with self.assertRaises(HTTPException) as denied:
                    studio.start_render(
                        studio.RenderRequest(
                            source="from manim import *\nclass MainScene(Scene):\n    pass",
                            scene="MainScene",
                        )
                    )
                self.assertEqual(denied.exception.status_code, 409)
                manager.begin_install.assert_called_once()
            finally:
                release.set()
                worker.join(5)
            studio.update_install_failed("Close failed")
            self.assertFalse(studio.UPDATE_INSTALLING)
            with (
                patch.object(
                    studio, "UPDATE_HANDLER", side_effect=OSError("Launch failed")
                ),
                self.assertLogs(level="ERROR"),
            ):
                with self.assertRaises(HTTPException) as failure:
                    studio.install_update()
                self.assertEqual(failure.exception.status_code, 503)
            self.assertFalse(studio.UPDATE_INSTALLING)

    def test_active_render_and_browser_install_are_rejected_before_handoff(self):
        manager = MagicMock()
        with (
            patch.object(studio, "UPDATES", manager),
            patch.object(studio, "ACTIVE", "active"),
            patch.object(studio, "UPDATE_INSTALLING", False),
        ):
            with self.assertRaises(HTTPException):
                studio.install_update()
            manager.begin_install.assert_not_called()
        with (
            patch.object(studio, "UPDATES", manager),
            patch.object(studio, "ACTIVE", None),
            patch.object(studio, "UPDATE_HANDLER", None),
            patch.object(studio, "UPDATE_INSTALLING", False),
        ):
            with self.assertRaises(HTTPException):
                studio.install_update()
            manager.begin_install.assert_not_called()


@unittest.skipUnless(
    os.name == "nt" and COMPILER.is_file(), "Windows .NET compiler required"
)
class NativeUpdateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.compiled = tempfile.TemporaryDirectory(prefix="studio-updater-build-")
        cls.tools = Path(cls.compiled.name).resolve()
        for name, references in (
            (
                "UpdateHelper",
                [
                    "System.IO.Compression.dll",
                    "System.IO.Compression.FileSystem.dll",
                    "System.Runtime.Serialization.dll",
                    "System.Web.Extensions.dll",
                ],
            ),
            ("Launcher", []),
        ):
            subprocess.run(
                [
                    str(COMPILER),
                    "/nologo",
                    "/target:winexe",
                    "/platform:x64",
                    "/reference:System.Windows.Forms.dll",
                    *[f"/reference:{item}" for item in references],
                    f"/out:{cls.tools / (name + '.exe')}",
                    str(ROOT / "packaging" / (name + ".cs")),
                ],
                check=True,
                capture_output=True,
            )
        stub = cls.tools / "stub.cs"
        stub.write_text(
            'using System; using System.IO; class Stub { static int Main() { File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"recovered.txt"),"old launcher restored"); return 0; } }'
        )
        cls.stub = cls.tools / "stub.exe"
        subprocess.run(
            [str(COMPILER), "/nologo", "/target:winexe", f"/out:{cls.stub}", str(stub)],
            check=True,
            capture_output=True,
        )

    @classmethod
    def tearDownClass(cls):
        cls.compiled.cleanup()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="studio-update-case-")
        self.addCleanup(self.temporary.cleanup)
        # The native host supplies resolved paths; Windows TEMP may use an 8.3 alias.
        self.base = Path(self.temporary.name).resolve()
        self.app = self.base / "Portable Studio"
        self.app.mkdir()
        for name, content in {
            "portable.mode": b"portable",
            "standalone.json": b'{"version":"1.0.0"}',
            "app/desktop.py": b"old code",
            "runtime/python.exe": b"old runtime",
            "UserData/.studio/preferences.json": b'{"manim-draft":"keep this"}',
            "UserData/outputs/video.mp4": b"keep video",
        }.items():
            target = self.app / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
        shutil.copy2(self.stub, self.app / "Manim Studio.exe")
        self.cache = (
            self.app / "UserData/.studio/updates" / ("install-" + uuid.uuid4().hex)
        )
        self.cache.mkdir(parents=True)
        self.helper = self.cache / "Studio Update.exe"
        shutil.copy2(self.tools / "UpdateHelper.exe", self.helper)

    def package(self, extra=None, metadata=None, missing=None):
        path = self.base / "update.zip"
        contents = {
            "app/desktop.py": b"new code",
            "runtime/python.exe": b"new runtime",
            "Manim Studio.exe": self.stub.read_bytes(),
            "standalone.json": b'{"version":"1.0.1"}',
            "portable.mode": b"portable",
        }
        for name in (
            "app/studio.py",
            "app/runtime.py",
            "app/worker.py",
            "app/updates.py",
            "app/update_handoff.py",
            "app/manim.cfg",
            "app/Studio Update.exe",
            "app/studio_ui/index.html",
            "app/studio_ui/app.js",
            "app/studio_ui/workspace.js",
            "app/studio_ui/updates.js",
            "app/studio_ui/style.css",
            "app/studio_ui/vendor/editor.bundle.js",
            "runtime/pythonw.exe",
            "runtime/python312.dll",
            "runtime/python312.zip",
            "runtime/python312._pth",
            "webview2/msedgewebview2.exe",
            "tools/ffmpeg.exe",
            "tools/ffprobe.exe",
            "math/texmfs/install/miktex/bin/x64/latex.exe",
            "math/texmfs/install/miktex/bin/x64/dvisvgm.exe",
            "LICENSE",
            "THIRD-PARTY-NOTICES.txt",
        ):
            contents[name] = b"fixture payload"
        if missing:
            contents.pop(missing)
        if metadata is not None:
            contents["standalone.json"] = metadata
        with ZipFile(path, "w") as archive:
            for name, data in contents.items():
                archive.writestr("Manim Studio/" + name, data)
            for name, data in (extra or {}).items():
                archive.writestr(name, data)
        return path

    def apply(self, artifact, digest=None, studio_pid=0, started=0):
        checksum = digest or hashlib.sha256(artifact.read_bytes()).hexdigest()
        return subprocess.run(
            [
                str(self.helper),
                "portable",
                str(self.app),
                str(artifact),
                checksum,
                "1.0.1",
                str(studio_pid),
                str(started),
                "0",
                "0",
                "--no-relaunch",
            ],
            capture_output=True,
            timeout=15,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )

    def assert_data_preserved(self):
        self.assertEqual(
            (self.app / "UserData/.studio/preferences.json").read_bytes(),
            b'{"manim-draft":"keep this"}',
        )
        self.assertEqual(
            (self.app / "UserData/outputs/video.mp4").read_bytes(), b"keep video"
        )

    def test_portable_update_preserves_user_data_and_cleans_transaction(self):
        self.assertEqual(
            self.apply(self.package()).returncode,
            0,
            (self.cache / "update.log").read_text(),
        )
        self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"new code")
        self.assert_data_preserved()
        self.assertFalse((self.app / "update.pending").exists())
        self.assertEqual(list(self.base.glob(".manim-update-*")), [])

    def test_portable_staging_supports_long_paths_for_valid_existing_locations(self):
        relative_length = 249 - len(str(self.app)) - 1
        relative = (
            "math/" + "n" * (relative_length - len("math//probe.txt")) + "/probe.txt"
        )
        final = self.app / relative
        incoming = (
            self.base
            / (".manim-update-" + self.cache.name.removeprefix("install-"))
            / "incoming"
            / relative
        )
        self.assertLess(len(str(final)), 260)
        self.assertGreater(len(str(incoming)), 260)
        artifact = self.package(
            extra={"Manim Studio/" + relative: b"long-path native payload"}
        )
        result = self.apply(artifact)
        self.assertEqual(result.returncode, 0, (self.cache / "update.log").read_text())
        self.assertEqual(final.read_bytes(), b"long-path native payload")
        self.assert_data_preserved()
        self.assertFalse((self.app / "update.pending").exists())
        self.assertEqual(list(self.base.glob(".manim-update-*")), [])

    def test_checksum_traversal_duplicate_and_user_data_entries_fail_before_ready(self):
        cases = [
            {"Manim Studio/../escape.txt": b"bad"},
            {"Manim Studio/UserData/preferences.json": b"bad"},
            {"Manim Studio/APP/desktop.py": b"duplicate"},
            {"Manim Studio/app/CON.txt": b"device"},
        ]
        for extra in cases:
            self.assertNotEqual(self.apply(self.package(extra)).returncode, 0)
            self.assertFalse((self.cache / "update.ready").exists())
            self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"old code")
            self.assert_data_preserved()
        self.assertNotEqual(self.apply(self.package(), "0" * 64).returncode, 0)

    def test_exact_process_identity_does_not_wait_for_reused_pid(self):
        self.assertEqual(
            self.apply(
                self.package(),
                studio_pid=os.getpid(),
                started=process_started_ticks() - 1,
            ).returncode,
            0,
        )

    def test_invalid_root_release_metadata_fails_before_ready(self):
        for metadata in (
            b'{"version":"1.0.1"',
            b'{"version":"1.0.0","nested":{"version":"1.0.1"}}',
            b'{"version":"1.0.0","version":"1.0.1"}',
        ):
            self.assertNotEqual(
                self.apply(self.package(metadata=metadata)).returncode, 0, metadata
            )
            self.assertFalse((self.cache / "update.ready").exists())
            self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"old code")
            self.assert_data_preserved()
        self.assertNotEqual(
            self.apply(self.package(missing="app/studio.py")).returncode, 0
        )
        self.assertFalse((self.cache / "update.ready").exists())

    def test_inactive_interrupted_preflight_is_reclaimed_and_active_owner_preserved(
        self,
    ):
        old = self.app / "UserData/.studio/updates" / ("install-" + uuid.uuid4().hex)
        old.mkdir()
        stage = self.base / (".manim-update-" + old.name.removeprefix("install-"))
        stage.mkdir()
        (stage / "payload").write_bytes(b"incomplete download extraction")
        marker = old / "stage.owner"
        marker.write_text(
            f"{self.app}\n{stage}\n{os.getpid()}\n{process_started_ticks()}\n"
        )
        self.assertEqual(self.apply(self.package()).returncode, 0)
        self.assertTrue(stage.exists(), "An active helper's stage must stay untouched")
        marker.write_text(
            f"{self.app}\n{stage}\n{os.getpid()}\n{process_started_ticks() - 1}\n"
        )
        self.assertEqual(self.apply(self.package()).returncode, 0)
        self.assertFalse(stage.exists())
        self.assertFalse(marker.exists())
        self.assert_data_preserved()

    def test_timeout_cleanup_is_bound_to_the_owned_preflight_stage(self):
        stage = self.base / (
            ".manim-update-" + self.cache.name.removeprefix("install-")
        )
        stage.mkdir()
        (stage / "payload").write_bytes(b"partial")
        marker = self.cache / "stage.owner"
        marker.write_text(f"{self.app}\n{stage}\n0\n0\n")
        cleanup_preflight(self.cache, self.app)
        self.assertFalse(stage.exists())
        self.assert_data_preserved()

    def test_locked_launcher_rolls_back_all_preceding_component_swaps(self):
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [
            wintypes.LPCWSTR,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.c_void_p,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.HANDLE,
        ]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.CreateFileW(
            str(self.app / "Manim Studio.exe"), 0x80000000, 1, None, 3, 0, None
        )
        self.assertNotEqual(handle, ctypes.c_void_p(-1).value)
        try:
            self.assertNotEqual(self.apply(self.package()).returncode, 0)
            self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"old code")
            self.assertEqual(
                (self.app / "runtime/python.exe").read_bytes(), b"old runtime"
            )
            self.assert_data_preserved()
        finally:
            kernel.CloseHandle(handle)
        result = subprocess.run(
            [str(self.helper), "recover", str(self.app), "--no-relaunch"],
            timeout=10,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.assertEqual(result.returncode, 0)
        self.assertFalse((self.app / "update.pending").exists())

    def test_partial_phase_marker_recovers_but_invalid_component_plan_is_rejected(self):
        stage = self.base / (".manim-update-" + uuid.uuid4().hex)
        backup = stage / "previous"
        backup.mkdir(parents=True)
        shutil.move(self.app / "app", backup / "app")
        (self.app / "app").mkdir()
        (self.app / "app/desktop.py").write_bytes(b"new")
        marker = self.app / "update.pending"
        header = f"MANIM-STUDIO-UPDATE-1\n{self.app}\n{stage}\n{self.helper}\n"
        marker.write_bytes((header + "app|1\ninvalid|1\ncomm").encode())
        command = [str(self.helper), "recover", str(self.app), "--no-relaunch"]
        self.assertNotEqual(
            subprocess.run(
                command, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW
            ).returncode,
            0,
        )
        self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"new")
        marker.write_bytes((header + "app|1\ncomm").encode())
        self.assertEqual(
            subprocess.run(
                command, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW
            ).returncode,
            0,
        )
        self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"old code")
        self.assert_data_preserved()

    def test_truncated_phase_remains_recoverable_after_a_second_rollback_failure(self):
        stage = self.base / (".manim-update-" + uuid.uuid4().hex)
        backup = stage / "previous"
        backup.mkdir(parents=True)
        shutil.copy2(self.stub, backup / "Manim Studio.exe")
        marker = self.app / "update.pending"
        marker.write_bytes(
            f"MANIM-STUDIO-UPDATE-1\n{self.app}\n{stage}\n{self.helper}\nManim Studio.exe|1\ncomm".encode()
        )
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.CreateFileW.argtypes = [
            wintypes.LPCWSTR,
            wintypes.DWORD,
            wintypes.DWORD,
            ctypes.c_void_p,
            wintypes.DWORD,
            wintypes.DWORD,
            wintypes.HANDLE,
        ]
        kernel.CreateFileW.restype = wintypes.HANDLE
        kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        handle = kernel.CreateFileW(
            str(self.app / "Manim Studio.exe"), 0x80000000, 1, None, 3, 0, None
        )
        self.assertNotEqual(handle, ctypes.c_void_p(-1).value)
        command = [str(self.helper), "recover", str(self.app), "--no-relaunch"]
        try:
            self.assertNotEqual(
                subprocess.run(
                    command, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW
                ).returncode,
                0,
            )
            self.assertTrue(marker.read_text().endswith("rollback\n"))
            self.assertNotIn("commrollback", marker.read_text())
        finally:
            kernel.CloseHandle(handle)
        self.assertEqual(
            subprocess.run(
                command, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW
            ).returncode,
            0,
        )
        self.assertFalse(marker.exists())
        self.assert_data_preserved()

    def test_prior_uncommitted_transaction_is_refused_before_closing_live_studio(self):
        marker = self.app / "update.pending"
        marker.write_bytes(b"uncommitted interrupted transaction")
        self.assertNotEqual(self.apply(self.package()).returncode, 0)
        self.assertFalse((self.cache / "update.ready").exists())
        self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"old code")
        self.assertEqual(marker.read_bytes(), b"uncommitted interrupted transaction")

    def test_actual_launcher_cannot_start_during_installer_handoff(self):
        installer_source = self.base / "setup.cs"
        installer_source.write_text(
            'using System; using System.IO; using System.Threading; class Setup { static int Main() { string root=AppDomain.CurrentDomain.BaseDirectory; File.WriteAllText(Path.Combine(root,"setup.entered"),"1"); while(!File.Exists(Path.Combine(root,"setup.release"))) Thread.Sleep(20); return 0; } }'
        )
        installer = self.cache / "setup.exe"
        subprocess.run(
            [
                str(COMPILER),
                "/nologo",
                "/target:winexe",
                f"/out:{installer}",
                str(installer_source),
            ],
            check=True,
            capture_output=True,
        )
        (self.app / "portable.mode").unlink()
        shutil.copy2(self.tools / "Launcher.exe", self.app / "Manim Studio.exe")
        checksum = hashlib.sha256(installer.read_bytes()).hexdigest()
        helper = subprocess.Popen(
            [
                str(self.helper),
                "installed",
                str(self.app),
                str(installer),
                checksum,
                "1.0.1",
                "0",
                "0",
                "0",
                "0",
                "--no-relaunch",
            ],
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        try:
            deadline = time.monotonic() + 5
            while (
                not (self.cache / "setup.entered").exists()
                and time.monotonic() < deadline
            ):
                time.sleep(0.02)
            self.assertTrue((self.cache / "setup.entered").exists())
            # Even if admission broke, this fixture never selects personal installed data.
            environment = {
                **os.environ,
                "MANIM_STUDIO_UPDATE_ROOT": str(self.app),
                "MANIM_STUDIO_UPDATE_DATA": str(self.base / "IsolatedData"),
            }
            result = subprocess.run(
                [str(self.app / "Manim Studio.exe")],
                env=environment,
                timeout=5,
                creationflags=subprocess.CREATE_NO_WINDOW,
            )
            self.assertEqual(result.returncode, 0)
            self.assertFalse(
                (self.base / "IsolatedData").exists(),
                "Launcher started while an update owned the singleton",
            )
            self.assert_data_preserved()
        finally:
            (self.cache / "setup.release").write_text("release")
            try:
                helper.wait(5)
            except subprocess.TimeoutExpired:
                helper.kill()
                helper.wait(5)
        self.assertEqual(helper.returncode, 0)

    def test_helper_relaunch_preserves_explicit_external_user_data(self):
        shutil.copy2(self.tools / "Launcher.exe", self.app / "Manim Studio.exe")
        stub = self.base / "data_probe.cs"
        stub.write_text(
            'using System; using System.IO; class Probe { static int Main() { File.WriteAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"data.txt"),Environment.GetEnvironmentVariable("MANIM_STUDIO_DATA") + "\\n" + Environment.GetEnvironmentVariable("MANIM_STUDIO_UPDATE_DATA")); return 0; } }'
        )
        python = self.app / "runtime/pythonw.exe"
        subprocess.run(
            [str(COMPILER), "/nologo", "/target:winexe", f"/out:{python}", str(stub)],
            check=True,
            capture_output=True,
        )
        (self.app / "webview2").mkdir()
        data = self.base / "ExternalData"
        result = subprocess.run(
            [str(self.helper), "recover", str(self.app), "0", "0"],
            env={**os.environ, "MANIM_STUDIO_DATA": str(data)},
            timeout=10,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(
            (self.app / "runtime/data.txt").read_text().splitlines(), [str(data)]
        )
        self.assert_data_preserved()

    def test_recovery_relaunches_after_running_launcher_exits_and_restores_image(self):
        stage = self.base / (".manim-update-" + uuid.uuid4().hex)
        backup = stage / "previous"
        backup.mkdir(parents=True)
        shutil.move(self.app / "app", backup / "app")
        (self.app / "app").mkdir()
        (self.app / "app/desktop.py").write_bytes(b"interrupted new code")
        shutil.copy2(self.stub, backup / "Manim Studio.exe")
        shutil.copy2(self.tools / "Launcher.exe", self.app / "Manim Studio.exe")
        (self.app / "update.pending").write_text(
            "MANIM-STUDIO-UPDATE-1\n"
            + str(self.app)
            + "\n"
            + str(stage)
            + "\n"
            + str(self.helper)
            + "\napp|1\nManim Studio.exe|1\n",
            encoding="utf-8",
        )
        process = subprocess.Popen(
            [str(self.app / "Manim Studio.exe")],
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        try:
            self.assertEqual(process.wait(10), 0)
            deadline = time.monotonic() + 10
            while (
                not (self.app / "recovered.txt").exists()
                and time.monotonic() < deadline
            ):
                time.sleep(0.05)
            self.assertTrue(
                (self.app / "recovered.txt").exists(),
                "Actual launcher did not hand off interrupted recovery",
            )
            self.assertEqual(
                (self.app / "Manim Studio.exe").read_bytes(), self.stub.read_bytes()
            )
            self.assertEqual((self.app / "app/desktop.py").read_bytes(), b"old code")
            self.assertFalse((self.app / "update.pending").exists())
            self.assert_data_preserved()
            # Recovery helper releases its image before temporary-directory cleanup.
            time.sleep(1.6)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(5)


if __name__ == "__main__":
    unittest.main()
