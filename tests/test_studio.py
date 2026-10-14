"""Real-render integration tests: uv run python -m unittest discover -s tests -v."""

import json
import asyncio
import ctypes
import os
from pathlib import Path
from fractions import Fraction
import re
import socket
import sys
import tempfile
import threading
import time
import unittest
import wave
from unittest.mock import MagicMock, patch
import urllib.error
import urllib.request
import uvicorn
import studio

SAMPLE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.add(Circle())\n        self.wait(0.2)\n"


class AnalysisTests(unittest.TestCase):
    def test_windows_safe_folder_names(self):
        self.assertEqual(studio.safe_folder_name("  My animation  "), "My animation")
        self.assertEqual(
            studio.safe_folder_name("Scene: circle / square?"),
            "Scene- circle - square-",
        )
        self.assertEqual(studio.safe_folder_name("CON.txt"), "_CON.txt")
        self.assertEqual(studio.safe_folder_name("LPT\u00b2"), "_LPT\u00b2")
        self.assertEqual(studio.safe_folder_name(" ... "), "Untitled animation")
        self.assertEqual(
            studio.safe_folder_name("Caf\u00e9 \u2605"), "Caf\u00e9 \u2605"
        )
        self.assertLessEqual(
            len(studio.safe_folder_name("\U0001f31f" * 100).encode("utf-16-le")), 200
        )
        self.assertFalse(
            re.search(
                r'[<>:"/\\|?*\x00-\x1f]', studio.safe_folder_name("../bad\\path. ")
            )
        )

    def test_folder_collisions_do_not_overwrite(self):
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(studio, "OUTPUTS", Path(directory)),
        ):
            first = studio.reserve_output_folder("My animation")
            (studio.OUTPUTS / first / "keep.txt").write_text("Keep this render")
            self.assertEqual(
                studio.reserve_output_folder("my ANIMATION"), "my ANIMATION (2)"
            )
            self.assertEqual(
                studio.reserve_output_folder("My animation"), "My animation (3)"
            )
            self.assertEqual(
                (studio.OUTPUTS / first / "keep.txt").read_text(), "Keep this render"
            )

    def test_markdown_and_multiple_scenes(self):
        result = studio.analyze(
            "```python\n" + SAMPLE + "\nclass Other(MainScene):\n    pass\n```"
        )
        self.assertEqual(result["scenes"], ["MainScene", "Other"])
        self.assertIsNone(result["error"])

    def test_alias_and_qualified_scene(self):
        self.assertEqual(
            studio.analyze("from manim import Scene as S\nclass Demo(S):\n    pass")[
                "scenes"
            ],
            ["Demo"],
        )
        self.assertEqual(
            studio.analyze("import manim as m\nclass Demo(m.ThreeDScene):\n    pass")[
                "scenes"
            ],
            ["Demo"],
        )

    def test_syntax_error_line(self):
        result = studio.analyze("from manim import *\nclass Bad(Scene)\n    pass")
        self.assertEqual(result["line"], 2)
        self.assertIn("Line 2", result["error"])

    def test_analysis_never_executes_code(self):
        self.assertIsNone(
            studio.analyze('raise RuntimeError("Do not run me")\n' + SAMPLE)["error"]
        )


class RenderIntegrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.original_outputs, cls.original_renders, cls.original_root = (
            studio.OUTPUTS,
            studio.RENDERS,
            studio.ROOT,
        )
        (
            cls.original_jobs,
            cls.original_active,
            cls.original_process,
            cls.original_worker,
        ) = (
            studio.JOBS,
            studio.ACTIVE,
            studio.PROCESS,
            studio.WORKER,
        )
        studio.JOBS, studio.ACTIVE, studio.PROCESS, studio.WORKER = {}, None, None, None
        studio.ROOT = Path(cls.temporary.name)
        studio.OUTPUTS = Path(cls.temporary.name) / "outputs"
        studio.OUTPUTS.mkdir()
        studio.RENDERS = Path(cls.temporary.name) / "renders"
        cls.socket = socket.socket()
        cls.socket.bind(("127.0.0.1", 0))
        cls.url = f"http://127.0.0.1:{cls.socket.getsockname()[1]}"
        cls.server = uvicorn.Server(uvicorn.Config(studio.app, log_level="error"))
        cls.thread = threading.Thread(
            target=lambda: cls.server.run(sockets=[cls.socket]), daemon=True
        )
        cls.thread.start()
        deadline = time.monotonic() + 10
        while not cls.server.started:
            if time.monotonic() > deadline:
                raise RuntimeError("Test server did not start")
            time.sleep(0.05)
        page = urllib.request.urlopen(cls.url).read().decode()
        cls.token = re.search(r'name="studio-token" content="([^"]+)"', page)[1]

    @classmethod
    def tearDownClass(cls):
        studio.shutdown_render()
        cls.server.should_exit = True
        cls.thread.join(timeout=10)
        cls.socket.close()
        studio.OUTPUTS, studio.RENDERS, studio.ROOT = (
            cls.original_outputs,
            cls.original_renders,
            cls.original_root,
        )
        studio.JOBS, studio.ACTIVE, studio.PROCESS, studio.WORKER = (
            cls.original_jobs,
            cls.original_active,
            cls.original_process,
            cls.original_worker,
        )
        cls.temporary.cleanup()

    def request(self, path, body=None, token=True, headers=None):
        request = urllib.request.Request(
            self.url + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={
                **(
                    {
                        "Content-Type": "application/json",
                        "X-Studio-Token": self.token if token else "",
                    }
                    if body is not None
                    else {}
                ),
                **(headers or {}),
            },
        )
        with urllib.request.urlopen(request, timeout=15) as response:
            return response.status, response.read(), response.headers

    def start(self, source=SAMPLE, quality="preview", name="Verification", **settings):
        return json.loads(
            self.request(
                "/api/render",
                {
                    "source": source,
                    "scene": "MainScene",
                    "quality": quality,
                    "name": name,
                    **settings,
                },
            )[1]
        )

    def wait(self, job):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            current = json.loads(self.request("/api/renders/" + job["id"])[1])
            if current["status"] in {"completed", "failed", "cancelled"}:
                while studio.ACTIVE and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertIsNone(
                    studio.ACTIVE, "Worker cleanup did not finish before the deadline"
                )
                return current
            time.sleep(0.1)
        self.fail("Render did not finish in 60 seconds")

    def wait_for_child(self, filename):
        path = studio.ROOT / filename
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if path.is_file():
                return int(path.read_text())
            time.sleep(0.02)
        self.fail("The scene did not start its test helper")

    def assert_child_stopped(self, pid):
        if os.name == "nt":
            from ctypes import wintypes

            kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel.OpenProcess.argtypes = [
                wintypes.DWORD,
                wintypes.BOOL,
                wintypes.DWORD,
            ]
            kernel.OpenProcess.restype = wintypes.HANDLE
            kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            kernel.WaitForSingleObject.restype = wintypes.DWORD
            kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            handle = kernel.OpenProcess(0x100000, False, pid)
            if handle:
                try:
                    self.assertEqual(
                        kernel.WaitForSingleObject(handle, 1000),
                        0,
                        "Render helper survived",
                    )
                finally:
                    kernel.CloseHandle(handle)
            else:
                self.assertEqual(
                    ctypes.get_last_error(), 87, "Could not inspect render helper"
                )
        else:
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    return
                status = Path(f"/proc/{pid}/stat")
                if (
                    sys.platform.startswith("linux")
                    and status.is_file()
                    and status.read_text().split()[2] == "Z"
                ):
                    return
                time.sleep(0.02)
            self.fail("Render helper survived")

    def descendant_script(
        self, filename, *, wait_for_cancel=False, redirect_output=False
    ):
        output = (
            ", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL"
            if redirect_output
            else ""
        )
        return (
            "from manim import *\nimport sys, subprocess, time\nfrom pathlib import Path\n"
            "class MainScene(Scene):\n    def construct(self):\n"
            f"        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)']{output})\n"
            f"        Path({filename!r}).write_text(str(child.pid))\n"
            + ("        time.sleep(60)\n" if wait_for_cancel else "")
            + "        self.wait(0.1)\n"
        )

    def test_01_mutation_token_and_invalid_script(self):
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request(
                "/api/render", {"source": SAMPLE, "scene": "MainScene"}, token=False
            )
        self.assertEqual(error.exception.code, 403)
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request(
                "/api/preferences",
                {"values": {}},
                headers={"X-Studio-Token": "\u00e9"},
            )
        self.assertEqual(error.exception.code, 403)
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.start("not valid python !!!")
        self.assertEqual(error.exception.code, 422)
        self.assertIsNone(studio.ACTIVE)

    def test_update_api_security_and_public_state(self):
        manager = studio.UpdateManager(
            "1.0.0", "installed", studio.ROOT / "package", studio.ROOT / "data"
        )
        self.addCleanup(manager.shutdown)
        private_path = studio.ROOT / "private-update-package.exe"
        # Include private handoff metadata so the public API checks a populated state.
        manager._ready = {"path": private_path, "sha256": "a" * 64}
        manager._release = {
            "version": "1.1.0",
            "url": "https://github.com/ItsTatsuya/manim-studio/releases/tag/v1.1.0",
            "asset": {"name": "Manim-Studio-1.1.0-Offline-Setup-x64.exe"},
        }
        manager._state["status"] = "ready"
        mocked = MagicMock(spec=studio.UpdateManager, wraps=manager)
        handler = MagicMock()
        with (
            patch.object(studio, "UPDATES", mocked),
            patch.object(studio, "UPDATE_HANDLER", handler),
            patch.object(studio, "UPDATE_INSTALLING", False),
            patch.object(studio, "ACTIVE", None),
        ):
            for action in ("check", "download", "cancel", "install"):
                with self.subTest(action=action):
                    request = urllib.request.Request(
                        self.url + "/api/updates/" + action,
                        data=b"{}",
                        headers={"Content-Type": "application/json"},
                    )
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        urllib.request.urlopen(request, timeout=15)
                    self.assertEqual(error.exception.code, 403)
            self.assertEqual(mocked.mock_calls, [])
            handler.assert_not_called()
            status, payload, _ = self.request("/api/updates")
            self.assertEqual(status, 200)
            public = json.loads(payload)
            self.assertEqual(public["status"], "ready")
            self.assertEqual(public["latest_version"], "1.1.0")
            self.assertNotIn(str(private_path), payload.decode())
            self.assertNotIn(str(studio.ROOT), payload.decode())
            self.assertTrue(
                {"path", "package_root", "data_root", "sha256"}.isdisjoint(public)
            )

        for edition in ("source", "installed", "portable"):
            with self.subTest(edition=edition):
                browser_manager = studio.UpdateManager(
                    "1.0.0", edition, studio.ROOT / "package", studio.ROOT / "data"
                )
                self.addCleanup(browser_manager.shutdown)
                blocked = MagicMock(spec=studio.UpdateManager, wraps=browser_manager)
                with (
                    patch.object(studio, "UPDATES", blocked),
                    patch.object(studio, "UPDATE_HANDLER", None),
                    patch.object(studio, "UPDATE_INSTALLING", False),
                    patch.object(studio, "ACTIVE", None),
                ):
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        self.request("/api/updates/install", {})
                    self.assertEqual(error.exception.code, 409)
                    blocked.begin_install.assert_not_called()
                    self.assertFalse(studio.UPDATE_INSTALLING)

    def test_preferences_persist_on_disk(self):
        self.request(
            "/api/preferences",
            {"values": {"manim-draft": SAMPLE, "manim-theme": "dark"}},
        )
        prefs = json.loads(self.request("/api/preferences")[1])
        self.assertEqual(prefs["manim-draft"], SAMPLE)
        self.assertEqual(prefs["manim-theme"], "dark")
        self.assertTrue((studio.ROOT / ".studio" / "preferences.json").is_file())

    def test_malformed_unicode_and_nested_source_return_validation_errors(self):
        malformed = chr(0xD800)
        for path, body in (
            ("/api/analyze", {"source": malformed}),
            ("/api/format", {"source": malformed}),
            (
                "/api/render",
                {"source": SAMPLE, "scene": "MainScene", "name": malformed},
            ),
            ("/api/preferences", {"values": {"manim-draft": malformed}}),
        ):
            with self.subTest(path=path):
                with self.assertRaises(urllib.error.HTTPError) as error:
                    self.request(path, body)
                self.assertEqual(error.exception.code, 422)
                detail = json.loads(error.exception.read())["detail"]
                self.assertIn("invalid", detail)
        self.assertIsNone(studio.ACTIVE)
        self.assertNotIn(malformed, studio.preferences().values())
        nested = "+" * 3000 + "1"
        result = json.loads(self.request("/api/analyze", {"source": nested})[1])
        self.assertIn("deeply nested", result["error"])
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request("/api/format", {"source": nested})
        self.assertEqual(error.exception.code, 422)

    def test_unserializable_validation_inputs_return_422_with_normal_errors_preserved(
        self,
    ):
        malformed = chr(0xD800)
        for path, body in (
            ("/api/analyze", {"source": {malformed: "invalid type"}}),
            (
                "/api/preferences",
                {"values": {"manim-draft": {malformed: "invalid type"}}},
            ),
            ("/api/analyze", {"source": float("nan")}),
            ("/api/preferences", {"values": {"manim-draft": float("inf")}}),
        ):
            with self.subTest(
                path=path,
                input_type=type(body.get("source", body.get("values"))).__name__,
            ):
                with self.assertRaises(urllib.error.HTTPError) as error:
                    self.request(path, body)
                self.assertEqual(error.exception.code, 422)
                detail = json.loads(error.exception.read())["detail"]
                self.assertIn("invalid text or numbers", detail)
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.request("/api/analyze", {"source": 123})
        self.assertEqual(error.exception.code, 422)
        details = json.loads(error.exception.read())["detail"]
        self.assertIsInstance(details, list)
        self.assertEqual(details[0]["loc"], ["body", "source"])
        self.assertEqual(details[0]["input"], 123)

    def test_02_all_qualities_and_video_range_requests(self):
        import av

        for quality, height, fps in [
            ("preview", 480, 15),
            ("standard", 720, 30),
            ("final", 1080, 60),
        ]:
            with self.subTest(quality=quality):
                job = self.wait(self.start(quality=quality))
                self.assertEqual(job["status"], "completed", job.get("log"))
                self.assertNotIn("width", job)
                self.assertNotIn("height", job)
                with av.open(
                    str(studio.OUTPUTS / job["folder"] / "video.mp4")
                ) as video:
                    self.assertEqual(video.streams.video[0].height, height)
                    rate = video.streams.video[0].average_rate
                    self.assertEqual(rate.numerator / rate.denominator, fps)
                    self.assertGreater(sum(1 for _ in video.decode(video=0)), 0)
                status, data, headers = self.request(
                    job["video"], headers={"Range": "bytes=0-99"}
                )
                self.assertEqual(status, 206)
                self.assertEqual(len(data), 100)
                self.assertIn("video/mp4", headers["Content-Type"])
                self.assertEqual(
                    self.request("/api/renders/" + job["id"] + "/script")[1]
                    .decode()
                    .splitlines(),
                    SAMPLE.strip().splitlines(),
                )
                self.assertTrue(
                    (studio.OUTPUTS / job["folder"] / "render.json").exists()
                )
                self.assertFalse((studio.RENDERS / job["id"]).exists())
                history = json.loads(self.request("/api/renders")[1])
                self.assertTrue(any(item["id"] == job["id"] for item in history))
                self.assertTrue(all("log" not in item for item in history))

    def test_custom_resolution_and_fractional_frame_rates_restore_in_history(self):
        import av

        for width, height, fps in (
            (640, 360, 17.5),
            (360, 640, 29.97),
            (320, 320, 12.75),
            (320, 240, 17.5000001),
        ):
            with self.subTest(width=width, height=height, fps=fps):
                job = self.wait(
                    self.start(
                        quality="standard",
                        name="Custom settings",
                        width=width,
                        height=height,
                        fps=fps,
                    )
                )
                self.assertEqual(job["status"], "completed", job.get("error"))
                self.assertEqual(
                    (job["width"], job["height"], job["resolution"]),
                    (width, height, f"{width}x{height}"),
                )
                rate = Fraction(str(fps))
                self.assertEqual(job["fps"], float(rate))
                folder = studio.OUTPUTS / job["folder"]
                with av.open(str(folder / "video.mp4")) as video:
                    self.assertEqual(
                        (video.streams.video[0].width, video.streams.video[0].height),
                        (width, height),
                    )
                    self.assertEqual(video.streams.video[0].average_rate, rate)
                    self.assertGreater(sum(1 for _ in video.decode(video=0)), 0)
                manifest = json.loads((folder / "render.json").read_text())
                self.assertEqual(
                    (manifest["width"], manifest["height"], manifest["fps"]),
                    (width, height, float(rate)),
                )
                self.assertNotIn("_requested_fps", manifest)
                self.assertNotIn("_requested_rate", manifest)
                studio.JOBS.pop(job["id"])

                async def reload():
                    async with studio.lifespan(studio.app):
                        self.assertEqual(
                            studio.JOBS[job["id"]]["resolution"], f"{width}x{height}"
                        )
                        self.assertEqual(studio.JOBS[job["id"]]["fps"], float(rate))

                asyncio.run(reload())
                status, data, _ = self.request(
                    job["video"], headers={"Range": "bytes=0-99"}
                )
                self.assertEqual((status, len(data)), (206, 100))

    def test_resolution_and_fps_overrides_are_independent_and_preserve_shortcuts(self):
        import av

        shortcut = studio.ROOT / "outputs" / "preview.mp4"
        shortcut.parent.mkdir(exist_ok=True)
        shortcut.write_bytes(b"Keep the previous legacy preview")
        for settings, expected in (
            ({"width": 320, "height": 240}, (320, 240, 15)),
            ({"fps": 17.5}, (854, 480, 17.5)),
        ):
            with self.subTest(settings=settings):
                job = self.wait(self.start(name="Independent settings", **settings))
                self.assertEqual(job["status"], "completed", job.get("error"))
                self.assertEqual((job["width"], job["height"], job["fps"]), expected)
                with av.open(
                    str(studio.OUTPUTS / job["folder"] / "video.mp4")
                ) as video:
                    self.assertEqual(
                        (
                            video.streams.video[0].width,
                            video.streams.video[0].height,
                            float(video.streams.video[0].average_rate),
                        ),
                        expected,
                    )
                self.assertEqual(
                    shortcut.read_bytes(), b"Keep the previous legacy preview"
                )

    def test_explicit_quality_is_normalized_and_only_matching_shortcut_updates(self):
        preview = studio.ROOT / "outputs" / "preview.mp4"
        final = studio.ROOT / "outputs" / "final.mp4"
        preview.parent.mkdir(exist_ok=True)
        preview.write_bytes(b"Keep preview")
        final.write_bytes(b"Keep final")
        custom = self.wait(
            self.start(
                quality="final",
                name="Custom final fallback",
                width=640,
                height=360,
                fps=17.5,
            )
        )
        self.assertEqual(custom["status"], "completed", custom.get("error"))
        self.assertEqual(custom["quality"], "standard")
        self.assertEqual(
            (preview.read_bytes(), final.read_bytes()), (b"Keep preview", b"Keep final")
        )
        known = self.wait(
            self.start(
                quality="preview",
                name="Explicit high settings",
                width=1920,
                height=1080,
                fps=60,
            )
        )
        self.assertEqual(known["status"], "completed", known.get("error"))
        self.assertEqual(known["quality"], "final")
        self.assertEqual(
            final.read_bytes(),
            (studio.OUTPUTS / known["folder"] / "video.mp4").read_bytes(),
        )
        self.assertEqual(preview.read_bytes(), b"Keep preview")

    def test_invalid_custom_settings_do_not_allocate_render_state(self):
        before = len(studio.JOBS)
        for settings in (
            {"width": 640},
            {"height": 360},
            {"width": True, "height": 360},
            {"width": 640.0, "height": 360},
            {"width": "640", "height": 360},
            {"width": 0, "height": 360},
            {"width": -2, "height": 360},
            {"width": 641, "height": 360},
            {"width": 640, "height": 361},
            {"width": 32768, "height": 2},
            {"width": 2, "height": 32768},
            {"fps": True},
            {"fps": 0},
            {"fps": -1},
            {"fps": "29.97"},
            {"fps": float("nan")},
            {"fps": float("inf")},
            {"fps": float("-inf")},
            {"fps": 2**31},
            {"fps": 5e-324},
            {"fps": 7e-7},
            {"fps": 1e-6},
            {"fps": 2e-6},
        ):
            with self.subTest(settings=settings):
                with self.assertRaises(urllib.error.HTTPError) as error:
                    self.start(**settings)
                self.assertEqual(error.exception.code, 422)
                self.assertIsNone(studio.ACTIVE)
                self.assertEqual(len(studio.JOBS), before)

    def test_custom_setting_preferences_persist_without_replacing_legacy_keys(self):
        self.request(
            "/api/preferences",
            {
                "values": {
                    "manim-resolution": "360x640",
                    "manim-fps": "29.97",
                    "manim-quality": "standard",
                }
            },
        )
        saved = json.loads(self.request("/api/preferences")[1])
        self.assertEqual(
            (saved["manim-resolution"], saved["manim-fps"], saved["manim-quality"]),
            ("360x640", "29.97", "standard"),
        )

    def test_fractional_frame_rate_with_audio_muxes_and_decodes(self):
        import av

        sound = studio.ROOT / "assets" / "custom-rate.wav"
        sound.parent.mkdir(exist_ok=True)
        with wave.open(str(sound), "wb") as audio:
            audio.setnchannels(1)
            audio.setsampwidth(2)
            audio.setframerate(16000)
            audio.writeframes(b"\0\0" * 6400)
        source = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.add_sound('assets/custom-rate.wav')\n        self.add(Circle())\n        self.wait(.4)\n"
        job = self.wait(
            self.start(
                source,
                quality="standard",
                name="Custom rate audio",
                width=320,
                height=240,
                fps=17.5,
            )
        )
        self.assertEqual(job["status"], "completed", job.get("error"))
        path = studio.OUTPUTS / job["folder"] / "video.mp4"
        with av.open(str(path)) as video:
            self.assertEqual(video.streams.video[0].average_rate, Fraction(35, 2))
            self.assertGreater(sum(1 for _ in video.decode(video=0)), 0)
        with av.open(str(path)) as video:
            self.assertTrue(video.streams.audio)
            self.assertGreater(sum(frame.samples for frame in video.decode(audio=0)), 0)

    def test_custom_frame_rate_adapter_preserves_script_override_and_cancel_recovery(
        self,
    ):
        import av

        override = "from manim import *\nconfig.frame_rate = 24\n" + SAMPLE
        job = self.wait(
            self.start(
                override,
                quality="standard",
                name="Scene frame rate override",
                width=320,
                height=240,
                fps=17.5,
            )
        )
        self.assertEqual(job["status"], "completed", job.get("error"))
        with av.open(str(studio.OUTPUTS / job["folder"] / "video.mp4")) as video:
            self.assertEqual(video.streams.video[0].average_rate, 24)
        self.assertEqual(job["fps"], 24)
        source = "from manim import *\nimport time\nclass MainScene(Scene):\n    def construct(self):\n        time.sleep(60)\n        self.wait(.2)\n"
        pending = self.start(
            source,
            quality="standard",
            name="Cancel custom settings",
            width=240,
            height=320,
            fps=17.5,
        )
        self.request("/api/renders/" + pending["id"] + "/cancel", {})
        self.assertEqual(self.wait(pending)["status"], "cancelled")
        recovered = self.wait(
            self.start(
                quality="standard",
                name="After custom cancel",
                width=240,
                height=320,
                fps=17.5,
            )
        )
        self.assertEqual(recovered["status"], "completed", recovered.get("error"))
        self.assertEqual(
            (recovered["width"], recovered["height"], recovered["fps"]),
            (240, 320, 17.5),
        )

    def test_03_runtime_failure_recovery(self):
        source = 'from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        raise ValueError("intentional test error")'
        job = self.wait(self.start(source))
        self.assertEqual(job["status"], "failed")
        self.assertIn("intentional test error", job["log"])
        self.assertEqual(job["problem"]["line"], 4)
        self.assertEqual(
            job["problem"]["message"], "ValueError: intentional test error"
        )
        self.assertIn("LLM", job["error"])
        self.assertNotIn("video", job)
        self.assertEqual(
            self.wait(self.start(name="After failure"))["status"], "completed"
        )

    def test_cancel_during_final_video_copy(self):
        original = studio.shutil.copy2

        def copy_and_cancel(source, destination, *args, **kwargs):
            result = original(source, destination, *args, **kwargs)
            if Path(destination).name == "video.tmp":
                studio.cancel_render(studio.ACTIVE)
            return result

        with patch.object(studio.shutil, "copy2", side_effect=copy_and_cancel):
            job = self.wait(self.start(name="Cancel at save"))
        self.assertEqual(job["status"], "cancelled")
        self.assertFalse((studio.OUTPUTS / job["folder"] / "render.json").exists())
        self.assertFalse((studio.OUTPUTS / job["folder"] / "video.mp4").exists())
        self.assertEqual(
            self.wait(self.start(name="After save cancellation"))["status"], "completed"
        )

    def test_04_cancel_and_concurrent_render_protection(self):
        source = "from manim import *\nimport time\nclass MainScene(Scene):\n    def construct(self):\n        time.sleep(20)\n        self.wait(1)"
        job = self.start(source)
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.start()
        self.assertEqual(error.exception.code, 409)
        self.request("/api/renders/" + job["id"] + "/cancel", {})
        self.assertEqual(self.wait(job)["status"], "cancelled")
        self.assertIsNone(studio.PROCESS)
        self.assertEqual(
            self.wait(self.start(name="After cancellation"))["status"], "completed"
        )

    def test_05_named_folders_survive_reload_and_legacy_links(self):
        first = self.wait(self.start(name="My animation"))
        second = self.wait(self.start(name="My animation"))
        self.assertEqual(
            (first["folder"], second["folder"]), ("My animation", "My animation (2)")
        )
        self.assertEqual(first["status"], "completed", first.get("log"))
        self.assertEqual(second["status"], "completed", second.get("log"))
        legacy_id = "a" * 32
        legacy_folder = studio.OUTPUTS / legacy_id
        legacy_folder.mkdir()
        legacy = {**first, "id": legacy_id, "video": f"/api/renders/{legacy_id}/video"}
        legacy.pop("folder")
        (legacy_folder / "render.json").write_text(json.dumps(legacy))
        (legacy_folder / "video.mp4").write_bytes(
            (studio.OUTPUTS / first["folder"] / "video.mp4").read_bytes()
        )
        studio.JOBS.pop(first["id"])

        async def reload_history():
            async with studio.lifespan(studio.app):
                self.assertEqual(studio.JOBS[first["id"]]["folder"], "My animation")
                self.assertEqual(studio.JOBS[legacy_id]["folder"], legacy_id)

        asyncio.run(reload_history())
        self.assertEqual(self.request(first["video"])[0], 200)
        self.assertEqual(self.request(legacy["video"])[0], 200)
        self.assertEqual(
            self.request("/api/renders/" + first["id"] + "/script")[0], 200
        )
        self.assertEqual(self.request("/api/renders/" + first["id"] + "/log")[0], 200)
        self.assertEqual(
            json.loads((studio.OUTPUTS / first["folder"] / "render.json").read_text())[
                "folder"
            ],
            first["folder"],
        )

    def test_worker_descendant_stdout_does_not_block_success(self):
        for redirect in (False, True):
            with self.subTest(redirect_output=redirect):
                filename = f"success-{redirect}-child.pid"
                before = time.monotonic()
                job = self.start(
                    self.descendant_script(filename, redirect_output=redirect),
                    name="Inherited helper output",
                )
                child = self.wait_for_child(filename)
                result = self.wait(job)
                self.assertEqual(result["status"], "completed", result.get("error"))
                self.assertLess(
                    time.monotonic() - before,
                    10,
                    "A finished render waited for its long-lived helper",
                )
                self.assert_child_stopped(child)

    def test_worker_descendant_cancel_and_shutdown_then_recovery(self):
        for action in ("cancel", "shutdown"):
            with self.subTest(action=action):
                filename = action + "-child.pid"
                job = self.start(
                    self.descendant_script(filename, wait_for_cancel=True),
                    name="Helper " + action,
                )
                child = self.wait_for_child(filename)
                before = time.monotonic()
                if action == "cancel":
                    self.request("/api/renders/" + job["id"] + "/cancel", {})
                else:
                    studio.shutdown_render()
                self.assertEqual(self.wait(job)["status"], "cancelled")
                self.assertLess(
                    time.monotonic() - before,
                    5,
                    "Cancellation waited for a long-lived helper",
                )
                self.assert_child_stopped(child)
                self.assertEqual(
                    self.wait(self.start(name="After helper " + action))["status"],
                    "completed",
                )


if __name__ == "__main__":
    unittest.main()
