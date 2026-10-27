"""Custom watermarks: settings, logo storage, inheritance and the video overlay."""

import io
import json
from pathlib import Path
import re
import shutil
import socket
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request

import uvicorn

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import projects  # noqa: E402
import studio  # noqa: E402
import watermark  # noqa: E402

SAMPLE = (
    "from manim import *\n"
    "class MainScene(Scene):\n"
    "    def construct(self):\n"
    "        self.add(Square(color=BLUE))\n"
    "        self.wait(0.4)\n"
)
SVG = (
    b'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50">'
    b'<rect x="0" y="0" width="50" height="50" fill="#ff0000"/>'
    b'<circle cx="75" cy="25" r="20" fill="#00ff00" stroke="#0000ff" stroke-width="4"/>'
    b"</svg>"
)


def png(color=(255, 0, 0, 255), size=(40, 20)) -> bytes:
    from PIL import Image

    output = io.BytesIO()
    Image.new("RGBA", size, color).save(output, format="PNG")
    return output.getvalue()


def make_video(path: Path, *, width=320, height=180, frames=24, audio=True):
    """A small black H.264 video, optionally with a sine-wave AAC track."""
    import av
    import numpy as np

    with av.open(str(path), "w", format="mp4") as output:
        video = output.add_stream("libx264", rate=24)
        video.width, video.height, video.pix_fmt = width, height, "yuv420p"
        sound = output.add_stream("aac", rate=44100, layout="mono") if audio else None
        black = np.zeros((height, width, 3), dtype=np.uint8)
        for _ in range(frames):
            for packet in video.encode(av.VideoFrame.from_ndarray(black, "rgb24")):
                output.mux(packet)
        for packet in video.encode(None):
            output.mux(packet)
        if sound is not None:
            samples = int(44100 * frames / 24)
            tone = (0.2 * np.sin(np.arange(samples) * 440 * 2 * np.pi / 44100)).astype(
                np.float32
            )
            for offset in range(0, samples, 1024):
                frame = av.AudioFrame.from_ndarray(
                    tone[offset : offset + 1024].reshape(1, -1),
                    format="flt",
                    layout="mono",
                )
                frame.sample_rate = 44100
                frame.pts = offset
                for packet in sound.encode(frame):
                    output.mux(packet)
            for packet in sound.encode(None):
                output.mux(packet)


def first_frame(path: Path):
    import av

    with av.open(str(path)) as source:
        return next(source.decode(video=0)).to_ndarray(format="rgb24")


def describe(path: Path) -> dict:
    import av

    with av.open(str(path)) as source:
        video = source.streams.video[0]
        frames = sum(1 for _ in source.decode(video=0))
    with av.open(str(path)) as source:
        audio = source.streams.audio[0] if source.streams.audio else None
        audio_packets = (
            [bytes(packet) for packet in source.demux(audio) if packet.size]
            if audio
            else []
        )
        return {
            "width": video.width,
            "height": video.height,
            "frames": frames,
            "audio_codec": audio.codec_context.name if audio else None,
            "audio_packets": audio_packets,
        }


class SettingsTests(unittest.TestCase):
    def test_validation_requires_content_for_the_chosen_type(self):
        self.assertEqual(watermark.Settings().type, "none")
        with self.assertRaises(ValueError):
            watermark.Settings(type="image")
        with self.assertRaises(ValueError):
            watermark.Settings(type="text", text="   ")
        with self.assertRaises(ValueError):
            watermark.Settings(type="text", text="a\nb")
        with self.assertRaises(ValueError):
            watermark.Settings(type="text", text="x", position="middle")
        with self.assertRaises(ValueError):
            watermark.Settings(type="text", text="x", opacity=1.5)
        with self.assertRaises(ValueError):
            watermark.Settings(type="image", logo="../../etc")
        self.assertIsNone(watermark.clean({"type": "image"}))
        self.assertIsNone(watermark.clean("text"))

    def test_placement_covers_every_position(self):
        size, margin = (20, 10), 5
        expected = {
            "top-left": (5, 5),
            "top-right": (75, 5),
            "bottom-left": (5, 85),
            "bottom-right": (75, 85),
            "center": (40, 45),
            "bottom-center": (40, 85),
        }
        for position, point in expected.items():
            self.assertEqual(
                watermark.placement(position, 100, 100, size, margin), point
            )
        self.assertEqual(set(expected), set(watermark.POSITIONS))


class StorageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def test_logos_are_rasterised_and_content_addressed(self):
        first = watermark.store_logo(self.root, png(), "Brand.PNG")
        again = watermark.store_logo(self.root, png(), "copy.png")
        self.assertEqual(first["logo"], again["logo"])
        self.assertEqual((first["width"], first["height"]), (40, 20))
        self.assertEqual(first["name"], "Brand.PNG")
        stored = watermark.logo_path(self.root, first["logo"])
        self.assertTrue(stored.is_file())
        self.assertTrue(stored.is_relative_to(watermark.logo_dir(self.root)))
        jpeg = io.BytesIO()
        from PIL import Image

        Image.new("RGB", (30, 30), (0, 0, 255)).save(jpeg, format="JPEG")
        self.assertEqual(
            watermark.store_logo(self.root, jpeg.getvalue(), "logo.jpg")["width"], 30
        )

    def test_svg_logos_are_drawn(self):
        from PIL import Image

        logo = watermark.store_logo(self.root, SVG, "logo.svg")
        with Image.open(watermark.logo_path(self.root, logo["logo"])) as image:
            self.assertEqual(image.mode, "RGBA")
            self.assertGreater(image.width, image.height)
            left = image.getpixel((image.width // 8, image.height // 2))
            self.assertGreater(left[0], 200)
            self.assertLess(left[1], 40)
            self.assertEqual(image.getpixel((image.width - 1, 0))[3], 0)

    def test_bad_logos_are_rejected_with_clear_messages(self):
        for data, name in (
            (b"not an image", "logo.png"),
            (png(), "logo.exe"),
            (b"", "logo.png"),
            (b"<svg xmlns='http://www.w3.org/2000/svg'></svg>", "logo.svg"),
            (png(color=(0, 0, 0, 0)), "blank.png"),
        ):
            with self.subTest(name=name), self.assertRaises(watermark.WatermarkError):
                watermark.store_logo(self.root, data, name)
        with self.assertRaises(KeyError):
            watermark.logo_path(self.root, "..\\escape")

    def test_default_settings_round_trip_and_damage_is_ignored(self):
        self.assertEqual(watermark.load_default(self.root), watermark.DEFAULT)
        saved = watermark.save_default(
            self.root, {"type": "text", "text": "© Studio", "position": "top-left"}
        )
        self.assertEqual(watermark.load_default(self.root), saved)
        watermark.default_path(self.root).write_text("{broken", encoding="utf-8")
        self.assertEqual(watermark.load_default(self.root), watermark.DEFAULT)

    def test_projects_and_video_workspaces_inherit_until_overridden(self):
        root = self.root / "projects"
        project = projects.create(root, "Course")
        video = projects.create_video(root, project["id"], "Lesson 1")
        default = watermark.save_default(self.root, {"type": "text", "text": "global"})

        def resolved(workspace=None):
            own, parent = projects.watermark_settings(root, project["id"], workspace)
            return watermark.effective(self.root, own, parent)

        self.assertEqual(resolved(), (default, "default"))
        self.assertEqual(resolved(video["workspace"]), (default, "default"))
        parent = watermark.Settings(type="text", text="course").model_dump()
        spoken = root / project["folder"] / projects.NARRATION_FILE
        spoken.write_text("Hello.", encoding="utf-8")
        before = spoken.stat().st_mtime_ns
        projects.set_watermark(root, project["id"], None, parent)
        # Only project.json changes, so a running video sees no file edit.
        self.assertEqual(spoken.stat().st_mtime_ns, before)
        self.assertEqual(spoken.read_text(encoding="utf-8"), "Hello.")
        self.assertEqual(resolved(), (parent, "own"))
        self.assertEqual(resolved(video["workspace"]), (parent, "project"))
        hidden = watermark.Settings(type="none").model_dump()
        projects.set_watermark(root, project["id"], video["workspace"], hidden)
        self.assertEqual(resolved(video["workspace"]), (hidden, "own"))
        # Ordinary project edits keep the watermark.
        projects.update(root, project["id"], name="Course 2", source="print(1)")
        projects.update(
            root, project["id"], workspace=video["workspace"], source="print(2)"
        )
        self.assertEqual(resolved(), (parent, "own"))
        self.assertEqual(resolved(video["workspace"]), (hidden, "own"))
        projects.set_watermark(root, project["id"], video["workspace"], None)
        self.assertEqual(resolved(video["workspace"]), (parent, "project"))
        stored = json.loads(
            (root / project["folder"] / "project.json").read_text(encoding="utf-8")
        )
        self.assertEqual(stored["watermark"], parent)

    def test_render_spec_needs_the_logo_file(self):
        self.assertIsNone(watermark.render_spec(self.root, None))
        self.assertIsNone(watermark.render_spec(self.root, {"type": "none"}))
        self.assertIsNone(
            watermark.render_spec(
                self.root, {"type": "text", "text": "x", "opacity": 0}
            )
        )
        missing = {"type": "image", "logo": "0" * 32}
        with self.assertRaises(watermark.WatermarkError):
            watermark.render_spec(self.root, missing)
        logo = watermark.store_logo(self.root, png(), "logo.png")["logo"]
        spec = watermark.render_spec(self.root, {"type": "image", "logo": logo})
        self.assertEqual(Path(spec["logo_file"]), watermark.logo_path(self.root, logo))


class OverlayTests(unittest.TestCase):
    """Runs the real overlay through the FFmpeg libraries Manim renders with."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def test_image_overlay_keeps_frames_size_and_audio(self):
        source, output = self.root / "in.mp4", self.root / "out.mp4"
        make_video(source)
        logo = watermark.store_logo(self.root, png((255, 255, 255, 255)), "w.png")
        spec = watermark.render_spec(
            self.root,
            {
                "type": "image",
                "logo": logo["logo"],
                "position": "top-left",
                "margin": 0,
                "scale": 25,
                "opacity": 1,
            },
        )
        stages = []
        watermark.apply(spec, source, output, report=stages.append)
        before, after = describe(source), describe(output)
        self.assertEqual(
            (after["width"], after["height"], after["frames"]),
            (before["width"], before["height"], before["frames"]),
        )
        self.assertEqual(after["audio_codec"], "aac")
        self.assertEqual(after["audio_packets"], before["audio_packets"])
        frame = first_frame(output)
        # 25% of 320 px wide, 2:1 logo: an 80x40 white block at the corner.
        self.assertGreater(frame[10, 10].min(), 230)
        self.assertGreater(frame[35, 75].min(), 230)
        self.assertLess(frame[60, 120].max(), 20)
        self.assertLess(frame[170, 310].max(), 20)
        self.assertTrue(any("%" in stage for stage in stages))
        self.assertEqual(stages[-1], "Adding your watermark · 100%")

    def test_text_overlay_is_drawn_with_opacity_and_silent_video_works(self):
        source, output = self.root / "in.mp4", self.root / "out.mp4"
        make_video(source, audio=False)
        spec = watermark.render_spec(
            self.root,
            {
                "type": "text",
                "text": "STUDIO",
                "font_size": 120,
                "color": "#ffffff",
                "position": "center",
                "opacity": 0.5,
            },
        )
        watermark.apply(spec, source, output)
        frame = first_frame(output)
        middle = frame[60:120, 80:240]
        self.assertGreater(middle.max(), 90)
        self.assertLess(middle.max(), 170)  # Half-transparent white over black.
        self.assertLess(frame[5, 5].max(), 20)
        self.assertIsNone(describe(output)["audio_codec"])

    def test_finish_render_replaces_the_newest_movie_in_place(self):
        media = self.root / "media"
        target = media / "videos" / "animation" / "480p15" / "video.mp4"
        target.parent.mkdir(parents=True)
        make_video(target)
        partial = target.parent / "partial_movie_files" / "MainScene" / "video.mp4"
        partial.parent.mkdir(parents=True)
        shutil.copy2(target, partial)
        spec = watermark.render_spec(
            self.root, {"type": "text", "text": "Hi", "opacity": 1}
        )
        file = self.root / "watermark.json"
        file.write_text(json.dumps({**spec, "media_dir": str(media)}), "utf-8")
        watermark.finish_render(file, report=lambda message: None)
        self.assertGreater(first_frame(target)[150:175, 240:315].max(), 200)
        self.assertLess(first_frame(partial).max(), 20)
        self.assertFalse((target.parent / "watermarked.mp4").exists())

    def test_unreadable_video_reports_a_watermark_error(self):
        media = self.root / "media" / "videos"
        media.mkdir(parents=True)
        (media / "video.mp4").write_bytes(b"not a video")
        file = self.root / "watermark.json"
        spec = watermark.render_spec(self.root, {"type": "text", "text": "Hi"})
        file.write_text(
            json.dumps({**spec, "media_dir": str(self.root / "media")}), "utf-8"
        )
        with self.assertRaises(watermark.WatermarkError):
            watermark.finish_render(file)


class WatermarkApiTests(unittest.TestCase):
    """The HTTP API and a real watermarked render through the worker process."""

    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.saved = (
            studio.OUTPUTS,
            studio.RENDERS,
            studio.ROOT,
            studio.JOBS,
            studio.ACTIVE,
            studio.PROCESS,
            studio.WORKER,
        )
        studio.JOBS, studio.ACTIVE, studio.PROCESS, studio.WORKER = {}, None, None, None
        studio.ROOT = Path(cls.temporary.name)
        studio.OUTPUTS = studio.ROOT / "outputs"
        studio.OUTPUTS.mkdir()
        studio.RENDERS = studio.ROOT / "renders"
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
        (
            studio.OUTPUTS,
            studio.RENDERS,
            studio.ROOT,
            studio.JOBS,
            studio.ACTIVE,
            studio.PROCESS,
            studio.WORKER,
        ) = cls.saved
        cls.temporary.cleanup()

    def request(self, path, body=None, *, raw=None, headers=None, token=True):
        data = raw if raw is not None else None
        all_headers = dict(headers or {})
        if body is not None:
            data = json.dumps(body).encode()
            all_headers["Content-Type"] = "application/json"
        if token:
            all_headers["X-Studio-Token"] = self.token
        request = urllib.request.Request(
            self.url + path,
            data=data,
            headers=all_headers,
            method="POST" if data is not None or token is not True else "GET",
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = response.read()
                kind = response.headers.get("content-type", "")
                return response.status, (
                    json.loads(payload) if "json" in kind else payload
                )
        except urllib.error.HTTPError as error:
            payload = error.read()
            try:
                return error.code, json.loads(payload or b"{}")
            except ValueError:
                return error.code, payload

    def get(self, path):
        with urllib.request.urlopen(self.url + path, timeout=30) as response:
            payload = response.read()
            kind = response.headers.get("content-type", "")
            return json.loads(payload) if "json" in kind else payload

    def upload(self, data, name):
        return self.request(
            "/api/watermark/logos",
            raw=data,
            headers={
                "X-Filename": name,
                "Content-Type": "application/octet-stream",
            },
        )

    def wait(self, job_id):
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            job = self.get(f"/api/renders/{job_id}")
            if job["status"] in {"completed", "failed", "cancelled"}:
                with studio.LOCK:
                    if studio.ACTIVE != job_id:
                        return job
            time.sleep(0.2)
        self.fail("Render did not finish")

    def test_api_requires_token_and_validates(self):
        status, _ = self.request(
            "/api/watermark", {"type": "text", "text": "x"}, token=False
        )
        self.assertEqual(status, 403)
        status, _ = self.request("/api/watermark", {"type": "text", "text": ""})
        self.assertEqual(status, 422)
        status, _ = self.request("/api/watermark", {"type": "image", "logo": "a" * 32})
        self.assertEqual(status, 422)
        status, _ = self.upload(b"x" * (watermark.MAX_LOGO_BYTES + 1), "big.png")
        self.assertEqual(status, 413)
        status, _ = self.upload(png(), "logo.html")
        self.assertEqual(status, 422)
        with self.assertRaises(urllib.error.HTTPError):
            self.get("/api/watermark/logos/..%5C..%5Cstudio.py")

    def test_default_project_and_video_settings_through_the_api(self):
        status, logo = self.upload(SVG, "brand%20mark.svg")
        self.assertEqual(status, 200, logo)
        self.assertEqual(logo["name"], "brand mark.svg")
        self.assertTrue(
            self.get(f"/api/watermark/logos/{logo['logo']}").startswith(b"\x89PNG")
        )
        default = {"type": "image", "logo": logo["logo"], "logo_name": logo["name"]}
        status, saved = self.request("/api/watermark", default)
        self.assertEqual(status, 200)
        self.assertEqual(self.get("/api/watermark")["settings"], saved["settings"])

        status, project = self.request("/api/projects", {"name": "Brand"})
        video = projects.create_video(studio.project_root(), project["id"], "Part 1")
        state = self.get(f"/api/projects/{project['id']}/watermark")
        self.assertEqual((state["source"], state["own"]), ("default", None))
        override = {"type": "text", "text": "Course"}
        status, state = self.request(
            f"/api/projects/{project['id']}/watermark", {"settings": override}
        )
        self.assertEqual((status, state["source"]), (200, "own"))
        path = f"/api/projects/{project['id']}/watermark?workspace={video['workspace']}"
        state = self.get(path)
        self.assertEqual(state["source"], "project")
        self.assertEqual(state["settings"]["text"], "Course")
        self.assertEqual(state["inherited_from"], "project")
        status, state = self.request(path, {"settings": {"type": "none"}})
        self.assertEqual((status, state["source"]), (200, "own"))
        status, state = self.request(path, {"settings": None})
        self.assertEqual(state["source"], "project")

        # Choose a logo from the video's assets.
        assets = projects.assets_dir(
            studio.project_root(), project["id"], video["workspace"]
        )
        (assets / "badge.png").write_bytes(png((0, 255, 0, 255)))
        status, chosen = self.request(
            f"/api/projects/{project['id']}/watermark/logo?workspace={video['workspace']}",
            {"path": "badge.png"},
        )
        self.assertEqual((status, chosen["name"]), (200, "badge.png"))
        status, _ = self.request(
            f"/api/projects/{project['id']}/watermark/logo",
            {"path": "../project.json"},
        )
        self.assertEqual(status, 404)
        status, _ = self.request(
            "/api/projects/" + "0" * 32 + "/watermark", {"settings": None}
        )
        self.assertEqual(status, 404)
        watermark.save_default(studio.ROOT, watermark.DEFAULT)

    def test_watermarked_render_and_manifest(self):
        status, project = self.request("/api/projects", {"name": "Marked"})
        status, logo = self.upload(png((255, 255, 255, 255), (60, 60)), "white.png")
        settings = {
            "type": "image",
            "logo": logo["logo"],
            "position": "bottom-left",
            "margin": 0,
            "scale": 20,
            "opacity": 1,
        }
        self.request(f"/api/projects/{project['id']}/watermark", {"settings": settings})
        status, job = self.request(
            "/api/render",
            {
                "source": SAMPLE,
                "scene": "MainScene",
                "project": project["id"],
                "quality": "preview",
            },
        )
        self.assertEqual(status, 202, job)
        self.assertTrue(job["watermarked"])
        self.assertNotIn("_watermark", job)
        finished = self.wait(job["id"])
        self.assertEqual(finished["status"], "completed", finished.get("error"))
        folder = studio.OUTPUTS / finished["folder"]
        frame = first_frame(folder / "video.mp4")
        height, width = frame.shape[:2]
        # A white square of 20% of the width sits in the bottom-left corner.
        self.assertGreater(frame[height - 5, 5].min(), 230)
        self.assertLess(frame[5, width - 5].max(), 40)
        manifest = json.loads((folder / "render.json").read_text(encoding="utf-8"))
        self.assertTrue(manifest["watermarked"])
        self.assertTrue(studio.valid_manifest(manifest))
        self.assertNotIn(str(studio.ROOT), json.dumps(manifest))

        # A missing logo is reported before anything is queued.
        watermark.logo_path(studio.ROOT, logo["logo"]).unlink()
        status, problem = self.request(
            "/api/render",
            {"source": SAMPLE, "scene": "MainScene", "project": project["id"]},
        )
        self.assertEqual(status, 422)
        self.assertIn("logo", problem["detail"])

    def test_watermark_failure_is_a_clear_render_error(self):
        # A damaged logo file passes the request check but fails in the worker.
        damaged = "f" * 32
        path = watermark.logo_path(studio.ROOT, damaged)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"not a png")
        status, job = self.request(
            "/api/render",
            {
                "source": SAMPLE,
                "scene": "MainScene",
                "watermark": {"type": "image", "logo": damaged},
            },
        )
        self.assertEqual(status, 202, job)
        finished = self.wait(job["id"])
        self.assertEqual(finished["status"], "failed")
        self.assertIn("watermark logo", finished["error"])
        self.assertFalse(finished.get("repairable", False))
        self.assertIn("watermark logo", finished["log"])


if __name__ == "__main__":
    unittest.main()
