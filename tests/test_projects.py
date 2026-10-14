"""Projects, per-project assets and narration (timeline, mixing, API and render)."""

import json
import asyncio
from concurrent.futures import ThreadPoolExecutor
import io
import os
from pathlib import Path
import re
import shutil
import socket
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import urllib.error
import urllib.parse
import urllib.request
from zipfile import ZipFile, ZipInfo
from document_fixtures import documents
from starlette.requests import ClientDisconnect

import av
import numpy as np
import uvicorn

import narration
import projects
import studio

SCENE = "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n        self.add(Circle())\n        self.wait(0.5)\n"


class ProjectStoreTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "projects"

    def test_create_update_list_and_delete(self):
        first = projects.create(self.root, "Intro: part 1?", "print(1)\n")
        second = projects.create(self.root, "intro- PART 1-")
        self.assertEqual(first["folder"], "Intro- part 1-")
        self.assertEqual(second["folder"], "intro- PART 1- (2)")
        self.assertTrue((self.root / first["folder"] / "assets").is_dir())
        projects.update(
            self.root,
            first["id"],
            name="Renamed",
            source="x = 2\n",
            narration={"enabled": True, "text": "Hi", "speed": 9, "voice": "../x"},
        )
        loaded = projects.read(self.root, first["id"])
        self.assertEqual(loaded["name"], "Renamed")
        self.assertEqual(loaded["source"], "x = 2\n")
        # Invalid narration values fall back to safe defaults.
        self.assertEqual(loaded["narration"]["speed"], 1.0)
        self.assertEqual(loaded["narration"]["voice"], "af_heart")
        self.assertTrue(loaded["narration"]["enabled"])
        self.assertEqual(
            projects.list_projects(self.root)[0]["id"], first["id"]
        )  # most recently updated first
        projects.delete(self.root, second["id"])
        replacement = projects.delete(self.root, first["id"])
        self.assertEqual(replacement["name"], "Untitled project")
        self.assertEqual(replacement["source"], "")
        self.assertEqual(projects.list_projects(self.root)[0]["id"], replacement["id"])
        with self.assertRaises(KeyError):
            projects.read(self.root, first["id"])
        with self.assertRaises(KeyError):
            projects.read(self.root, "0" * 32)
        with self.assertRaises(KeyError):
            projects.read(self.root, "../outside")

    def test_damaged_projects_are_ignored(self):
        projects.create(self.root, "Good")
        (self.root / "Broken").mkdir()
        (self.root / "Broken" / "project.json").write_text("{not json")
        (self.root / "Wrong").mkdir()
        (self.root / "Wrong" / "project.json").write_text('{"id": "x"}')
        self.assertEqual(
            [item["name"] for item in projects.list_projects(self.root)], ["Good"]
        )

    def test_failed_last_project_delete_does_not_leave_an_extra_blank_project(self):
        project = projects.create(self.root, "Keep", SCENE)
        target = projects.folder(self.root, project["id"])
        real_delete = shutil.rmtree

        def delete(path, **kwargs):
            if path == target:
                raise PermissionError("File is open")
            return real_delete(path, **kwargs)

        with patch("projects.shutil.rmtree", side_effect=delete):
            with self.assertRaisesRegex(
                projects.ProjectError, "Close any open project files"
            ):
                projects.delete(self.root, project["id"])
        self.assertEqual(len(projects.list_projects(self.root)), 1)
        self.assertEqual(projects.read(self.root, project["id"])["source"], SCENE)

    def test_first_run_migrates_draft_and_legacy_assets(self):
        legacy = Path(self.temporary.name) / "assets"
        (legacy / "images").mkdir(parents=True)
        (legacy / "README.txt").write_text("old instructions")
        (legacy / "logo.png").write_bytes(b"png")
        (legacy / "images" / "a.svg").write_text("<svg/>")
        projects.ensure_default(
            self.root, legacy, {"manim-draft": SCENE, "manim-name": "Old work"}
        )
        [project] = projects.list_projects(self.root)
        self.assertEqual(project["name"], "Old work")
        self.assertEqual(projects.read(self.root, project["id"])["source"], SCENE)
        assets = self.root / project["folder"] / "assets"
        self.assertEqual(
            sorted(item["path"] for item in projects.list_assets(assets)),
            ["images/a.svg", "logo.png"],
        )
        # Never runs twice.
        projects.ensure_default(self.root, legacy, {"manim-draft": "other"})
        self.assertEqual(len(projects.list_projects(self.root)), 1)

    def test_asset_names_are_safe_and_never_overwrite(self):
        directory = Path(self.temporary.name) / "assets"
        first = projects.reserve_asset(directory, "..\\..\\evil:name?.png")
        self.assertEqual(first.parent, directory)
        self.assertEqual(first.name, "evil-name-.png")
        first.write_bytes(b"1")
        self.assertEqual(
            projects.reserve_asset(directory, "evil-name-.png").name,
            "evil-name- (2).png",
        )
        for bad in ("../x.png", "", "a/../../b", "..\\x"):
            with self.assertRaises(KeyError):
                projects.asset_file(directory, bad)
        self.assertEqual(projects.asset_kind(Path("x.MP3")), "audio")
        self.assertEqual(projects.asset_kind(Path("x.svg")), "vector")

    def package(self, files):
        return [
            (
                path,
                io.BytesIO(content.encode() if isinstance(content, str) else content),
                len(content.encode() if isinstance(content, str) else content),
            )
            for path, content in files.items()
        ]

    def test_folder_import_keeps_nested_assets_and_needed_helpers(self):
        source = SCENE.replace(
            "from manim import *",
            "from manim import *\nfrom helpers.palette import COLOR",
        )
        result = projects.import_package(
            self.root,
            self.package(
                {
                    "Pitch/pitch.py": source,
                    "Pitch/helpers/__init__.py": "",
                    "Pitch/helpers/palette.py": "COLOR = 'BLUE'",
                    "Pitch/assets/fonts/LICENSE.txt": "font license",
                    "Pitch/assets/images/logo.svg": "<svg/>",
                    "Pitch/narration.json": json.dumps(
                        [
                            {"chapter": 1, "start": 0, "text": "Hello"},
                            {"start": 18, "text": "Next"},
                        ]
                    ),
                    "Pitch/assemble_narration.py": "raise RuntimeError('must never execute')",
                    "Pitch/preview.mp4": b"generated output",
                }
            ),
        )
        path = self.root / result["folder"]
        self.assertEqual(result["name"], "Pitch")
        self.assertEqual((path / "script.py").read_text(), source)
        self.assertEqual((path / "helpers/palette.py").read_text(), "COLOR = 'BLUE'")
        self.assertTrue((path / "helpers/__init__.py").is_file())
        self.assertEqual(
            (path / "narration.txt").read_text(), "[0s] Hello\n\n[18s] Next"
        )
        self.assertEqual(
            [item["path"] for item in projects.list_assets(path / "assets")],
            ["fonts/LICENSE.txt", "images/logo.svg"],
        )
        self.assertFalse((path / "preview.mp4").exists())
        self.assertFalse((path / "assemble_narration.py").exists())
        self.assertTrue(result["narration"]["enabled"])
        # Narration remains editable as a file and survives an ordinary save.
        (path / "narration.txt").write_text("[0:03] Edited")
        projects.update(self.root, result["id"], name="Renamed")
        self.assertEqual(
            projects.read(self.root, result["id"])["narration"]["text"], "[0:03] Edited"
        )

    def test_zip_import_manifest_and_non_overwriting_repeat(self):
        stream = io.BytesIO()
        with ZipFile(stream, "w") as archive:
            archive.writestr("outer/Pitch/main.py", SCENE)
            archive.writestr("outer/Pitch/other.py", SCENE)
            archive.writestr(
                "outer/Pitch/project.json",
                json.dumps({"name": "My pitch", "entrypoint": "main.py"}),
            )
            archive.writestr("outer/Pitch/narration.txt", "[0:00] A circle.")
            archive.writestr("outer/Pitch/.DS_Store", "ignored")
        for number in (1, 2):
            result = projects.import_zip(self.root, stream)
            self.assertEqual(
                result["folder"], "My pitch" if number == 1 else "My pitch (2)"
            )
            self.assertTrue(result["narration"]["enabled"])
        self.assertEqual(len(projects.list_projects(self.root)), 2)

    def test_package_rejections_leave_no_partial_project(self):
        cases = [
            {"script.py": SCENE, "assets/../escape.txt": "x"},
            {"script.py": SCENE, "C:/escape.txt": "x"},
            {"script.py": SCENE, "/absolute.txt": "x"},
            {"script.py": SCENE, "assets/NUL.txt": "x"},
            {"script.py": SCENE, "assets/logo.png": "x", "assets/LOGO.png": "y"},
            {"one.py": SCENE, "two.py": SCENE},
            {"script.py": SCENE, "narration.json": "{broken"},
            {"script.py": SCENE, "narration.json": '[{"start":-0.5,"text":"bad"}]'},
            {"script.py": SCENE, "narration.json": '[{"start":NaN,"text":"bad"}]'},
        ]
        for files in cases:
            with (
                self.subTest(files=list(files)),
                self.assertRaises(projects.ProjectError),
            ):
                projects.import_package(self.root, self.package(files))
            self.assertEqual(projects.list_projects(self.root), [])
        oversized = [
            ("script.py", io.BytesIO(SCENE.encode()), projects.MAX_PACKAGE_BYTES + 1)
        ]
        with self.assertRaises(projects.ProjectError):
            projects.import_package(self.root, oversized)
        with self.assertRaises(projects.ProjectError):
            projects.import_zip(self.root, io.BytesIO(b"not a zip"))
        stream = io.BytesIO()
        with ZipFile(stream, "w") as archive:
            link = ZipInfo("assets/link")
            link.create_system, link.external_attr = 3, 0o120777 << 16
            archive.writestr("script.py", SCENE)
            archive.writestr(link, "../../outside")
        with self.assertRaises(projects.ProjectError):
            projects.import_zip(self.root, stream)
        # A disk failure after staging began must not expose an incomplete project.
        with patch("projects.shutil.copyfileobj", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                projects.import_package(
                    self.root, self.package({"script.py": SCENE, "assets/a.svg": "svg"})
                )
        self.assertEqual(list(self.root.iterdir()), [])

    def test_recorded_audio_does_not_enable_a_second_voice(self):
        result = projects.import_package(
            self.root,
            self.package(
                {
                    "script.py": SCENE,
                    "narration.txt": "A circle.",
                    "assets/audio/narration.wav": b"recording",
                }
            ),
        )
        self.assertFalse(result["narration"]["enabled"])
        self.assertIn("doubling", result["warnings"][0])


class FakeEngine:
    """Stands in for Kokoro: 0.5 s of tone per word."""

    def __init__(self):
        self.calls = []

    def create(self, text, voice, speed, lang, **options):
        self.calls.append((text, options))
        seconds = 0.5 * len(text.split()) / speed
        count = int(seconds * narration.SAMPLE_RATE)
        tone = 0.2 * np.sin(np.arange(count) * 2 * np.pi * 220 / narration.SAMPLE_RATE)
        return tone.astype(np.float32), narration.SAMPLE_RATE


def _tone(seconds: float):
    count = int(seconds * narration.SAMPLE_RATE)
    wave = 0.2 * np.sin(np.arange(count) * 2 * np.pi * 220 / narration.SAMPLE_RATE)
    return wave.astype(np.float32)


def write_video(path: Path, seconds: float, with_audio: bool):
    with av.open(str(path), "w") as output:
        video = output.add_stream("libx264", rate=10)
        video.width, video.height, video.pix_fmt = 64, 64, "yuv420p"
        audio = output.add_stream("aac", rate=48000) if with_audio else None
        for index in range(int(seconds * 10)):
            frame = av.VideoFrame.from_ndarray(
                np.full((64, 64, 3), index * 3 % 255, dtype=np.uint8), format="rgb24"
            )
            for packet in video.encode(frame):
                output.mux(packet)
        for packet in video.encode(None):
            output.mux(packet)
        if audio is not None:
            samples = np.full((1, int(48000 * seconds)), 0.1, dtype=np.float32)
            frame = av.AudioFrame.from_ndarray(samples, format="flt", layout="mono")
            frame.sample_rate = 48000
            for packet in audio.encode(frame):
                output.mux(packet)
            for packet in audio.encode(None):
                output.mux(packet)


class NarrationTests(unittest.TestCase):
    def test_timeline_parsing(self):
        segments = narration.parse_timeline(
            "# note, not spoken\nFirst line\ncontinues here.\n\n"
            "[0:02.5] Pinned.\n[0:59:03] Hours work.\n[7s] Seconds.\n\n\n"
        )
        self.assertEqual(
            [(item["start"], item["text"]) for item in segments],
            [
                (None, "First line continues here."),
                (2.5, "Pinned."),
                (3543.0, "Hours work."),
                (7.0, "Seconds."),
            ],
        )
        self.assertEqual(narration.parse_timeline("# only notes\n\n"), [])
        with self.assertRaises(narration.NarrationError):
            narration.parse_timeline("\n\n".join(["word"] * 401))

    def test_long_paragraphs_are_chunked_at_sentences(self):
        sentence = "The derivative measures how fast a function changes. "
        chunks = narration.split_text(sentence * 40)
        self.assertGreater(len(chunks), 1)
        self.assertTrue(all(len(chunk) <= narration.CHUNK_CHARS for chunk in chunks))
        self.assertTrue(all(chunk.endswith(".") for chunk in chunks))
        self.assertEqual(" ".join(chunks).split(), (sentence * 40).split())
        # Text with no punctuation or spaces still splits instead of failing.
        giant = narration.split_text("a" * 1000)
        self.assertTrue(all(len(chunk) <= narration.CHUNK_CHARS for chunk in giant))
        self.assertEqual("".join(giant), "a" * 1000)

    def test_long_paragraph_works_without_duration_output(self):
        class Plain:
            """Like the bundled Kokoro models: `continuous=True` is unsupported."""

            def __init__(self):
                self.calls = []

            def create(self, text, **options):
                if options.get("continuous"):
                    raise ValueError(
                        "continuous synthesis needs a model that reports durations"
                    )
                self.calls.append(text)
                return _tone(0.2), narration.SAMPLE_RATE

        engine = Plain()
        paragraph = "A sentence that keeps going and going. " * 30
        track, _ = narration.synthesize(
            [{"start": None, "text": paragraph, "line": 1}],
            voice="af_heart",
            speed=1.0,
            engine=engine,
        )
        self.assertGreater(len(engine.calls), 1)
        self.assertGreater(track.size, 0)

    def test_unspeakable_text_is_skipped_with_a_warning(self):
        class Silent:
            def create(self, text, **options):
                raise ValueError(f"No phonemes of {text!r} are in the model vocabulary")

        track, warnings = narration.synthesize(
            [{"start": None, "text": "...", "line": 4}],
            voice="af_heart",
            speed=1.0,
            engine=Silent(),
        )
        self.assertEqual(track.size, 0)
        self.assertIn("line 4", warnings[0])

    def test_finished_lines_are_checkpointed(self):
        import tempfile

        class Counting:
            calls = 0

            def create(self, text, **options):
                Counting.calls += 1
                return _tone(0.2), narration.SAMPLE_RATE

        segments = narration.parse_timeline("One line.\n\nSecond line.")
        with tempfile.TemporaryDirectory() as folder:
            for _ in range(2):
                narration.synthesize(
                    segments,
                    voice="af_heart",
                    speed=1.0,
                    engine=Counting(),
                    cache_dir=folder,
                )
            self.assertEqual(Counting.calls, 2)
            # A changed line is the only one generated again.
            changed = narration.parse_timeline("One line.\n\nSecond line, edited.")
            narration.synthesize(
                changed,
                voice="af_heart",
                speed=1.0,
                engine=Counting(),
                cache_dir=folder,
            )
            self.assertEqual(Counting.calls, 3)

    def test_video_checkpoint_round_trip(self):
        import tempfile

        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            media = root / "media" / "videos" / "x"
            media.mkdir(parents=True)
            (media / "video.mp4").write_bytes(b"silent animation")
            spec = {
                "media_dir": str(root / "media"),
                "video_cache": str(root / "cache" / "videos" / "abc"),
            }
            narration.checkpoint_video(spec)
            fresh = {**spec, "media_dir": str(root / "second")}
            self.assertTrue(narration.restore_video(fresh, report=lambda m: None))
            restored = list((root / "second").rglob("video.mp4"))
            self.assertEqual(restored[0].read_bytes(), b"silent animation")
            self.assertFalse(
                narration.restore_video(
                    {**fresh, "video_cache": str(root / "cache" / "videos" / "none")}
                )
            )

    def test_voices_and_languages(self):
        voices = {voice["id"]: voice for voice in narration.voices()}
        self.assertEqual(voices["af_heart"]["lang"], "en-us")
        self.assertEqual(voices["bm_george"]["language"], "British English")
        self.assertEqual(voices["bm_george"]["gender"], "Male")
        with self.assertRaises(narration.NarrationError):
            narration.language("zz_unknown")

    def test_synthesis_places_lines_and_reports_overlaps(self):
        segments = narration.parse_timeline("one two\n\n[0:00.5] three\n\n[5] four")
        engine = FakeEngine()
        track, warnings = narration.synthesize(
            segments,
            voice="af_heart",
            speed=1.0,
            engine=engine,
            report=lambda m: None,
        )
        rate = narration.SAMPLE_RATE
        # 1 s, then a pinned line that overlaps (moved to 1 s), then one at 5 s.
        self.assertEqual(len(warnings), 1)
        self.assertIn("previous line is still speaking", warnings[0])
        # Mastering adds only a short room tail after the last line.
        self.assertAlmostEqual(len(track) / rate, 5.5, delta=0.25)
        self.assertGreater(abs(track[int(1.2 * rate)]), 0)
        self.assertLess(abs(track[int(3 * rate)]), 1e-4)
        self.assertLessEqual(float(np.max(np.abs(track))), 0.95)
        self.assertTrue(
            all(options["sentence_pause"] > 0.25 for _, options in engine.calls)
        )

    def test_speech_text_reads_notation_aloud(self):
        cases = {
            "x^2 + y^2 = r^2, e.g. here.": "x squared plus y squared equals r squared, for example, here.",
            "f(x) = √x for x ≥ 0": "f of x equals the square root of x for x is greater than or equal to 0",
            "Area is πr² (roughly) — neat": "Area is pi r squared, roughly, neat",
            "half is 1/2 and 5/7 too": "half is one half and 5 over 7 too",
            "as n → ∞ we see **it**": "as n approaches infinity we see it",
        }
        for text, spoken in cases.items():
            self.assertEqual(narration.prepare_text(text), spoken)

    def test_mastering_levels_and_limits(self):
        rate = narration.SAMPLE_RATE
        quiet = 0.25 * np.sin(np.arange(rate) * 2 * np.pi * 200 / rate)
        loud = 0.9 * np.sin(np.arange(rate) * 2 * np.pi * 200 / rate)
        track = np.concatenate([quiet, np.zeros(rate // 2), loud]).astype(np.float32)
        result = narration.master(track)
        self.assertLessEqual(float(np.max(np.abs(result))), 0.95)
        first = np.sqrt(np.mean(result[rate // 4 : 3 * rate // 4] ** 2))
        second = np.sqrt(np.mean(result[7 * rate // 4 : 9 * rate // 4] ** 2))
        # 10 dB apart before mastering; 2:1 leveling roughly halves the gap.
        self.assertLess(20 * np.log10(second / first), 7)
        self.assertEqual(narration.master(np.zeros(0, np.float32)).size, 0)

    def test_mux_copies_video_and_mixes_existing_audio(self):
        with tempfile.TemporaryDirectory() as folder:
            for with_audio in (False, True):
                source = Path(folder) / f"in-{with_audio}.mp4"
                output = Path(folder) / f"out-{with_audio}.mp4"
                write_video(source, 2.0, with_audio)
                track = np.full(int(narration.SAMPLE_RATE * 3), 0.2, dtype=np.float32)
                result = narration.mux(source, track, output)
                self.assertAlmostEqual(result["video"], 2.0, places=1)
                with av.open(str(output)) as video:
                    self.assertEqual(len(video.streams.video), 1)
                    self.assertEqual(video.streams.video[0].codec_context.name, "h264")
                    frames = sum(1 for _ in video.decode(video=0))
                    self.assertEqual(frames, 20)
                with av.open(str(output)) as video:
                    samples = np.concatenate(
                        [frame.to_ndarray() for frame in video.decode(audio=0)], axis=1
                    )
                self.assertGreaterEqual(samples.shape[1] / 48000, 2.95)
                # Mixed audio is louder where the original had sound.
                early = float(np.abs(samples[:, 48000 : 48000 + 4800]).mean())
                late = float(
                    np.abs(samples[:, 2 * 48000 + 9600 : 2 * 48000 + 14400]).mean()
                )
                if with_audio:
                    self.assertGreater(early, late + 0.05)
                else:
                    self.assertAlmostEqual(early, late, places=1)

    def test_downloader_verifies_hash_and_cleans_partial_files(self):
        with tempfile.TemporaryDirectory() as folder:
            downloader = narration.ModelDownloader(Path(folder))
            payload = b"not the model"

            class Response:
                def __init__(self):
                    self.chunks = [payload]

                def read(self, size):
                    return self.chunks.pop() if self.chunks else b""

                def __enter__(self):
                    return self

                def __exit__(self, *args):
                    return False

            with patch.object(
                narration.urllib.request, "urlopen", return_value=Response()
            ):
                downloader.start("standard")
                downloader.thread.join(timeout=10)
            state = downloader.snapshot()
            self.assertEqual(state["status"], "failed")
            self.assertIn("failed verification", state["error"])
            self.assertEqual(list(narration.model_dir(Path(folder)).iterdir()), [])
            self.assertFalse(narration.installed(Path(folder), "standard"))
            self.assertEqual(
                narration.download_size(Path(folder), "standard"),
                narration.FILES["voices"]["size"] + narration.FILES["standard"]["size"],
            )


class ProjectApiTests(unittest.TestCase):
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
        studio.RENDERS = studio.ROOT / "renders"
        models = os.environ.get("MANIM_STUDIO_TEST_KOKORO")
        if models:
            target = narration.model_dir(studio.ROOT)
            target.mkdir(parents=True)
            for item in (narration.FILES["voices"], narration.FILES["standard"]):
                shutil.copy2(Path(models) / item["name"], target / item["name"])
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

    def call(self, path, body=None, method=None, headers=None, raw=None):
        data = (
            raw
            if raw is not None
            else None
            if body is None
            else json.dumps(body).encode()
        )
        request = urllib.request.Request(
            self.url + path,
            data=data,
            method=method or ("GET" if data is None else "POST"),
            headers={
                "Content-Type": "application/json",
                "X-Studio-Token": self.token,
                **(headers or {}),
            },
        )
        with urllib.request.urlopen(request) as response:
            payload = response.read()
            return (
                json.loads(payload)
                if response.headers["content-type"].startswith("application/json")
                else payload
            )

    def wait(self, job_id):
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            job = self.call(f"/api/renders/{job_id}")
            if job["status"] in {"completed", "failed", "cancelled"}:
                while studio.ACTIVE and time.monotonic() < deadline:
                    time.sleep(0.05)
                return job
            time.sleep(0.2)
        self.fail("Render did not finish")

    def test_first_run_creates_a_project(self):
        self.assertGreaterEqual(len(self.call("/api/projects")["projects"]), 1)

    def test_native_picker_batch_preserves_bytes_and_continues_after_failure(self):
        project = self.call("/api/projects", {"name": "Native uploads"})
        route = f"/api/projects/{project['id']}/assets/choose"
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            chosen = []
            for document in documents():
                file = folder / document["name"]
                file.write_bytes(document["buffer"])
                chosen.append(file)
            missing = folder / "missing.pdf"
            with patch("studio.file_picker.choose", return_value=[missing, *chosen]):
                result = self.call(route, {"kind": "documents"})
            self.assertEqual(len(result["assets"]), 4)
            self.assertEqual(result["errors"][0]["file"], missing.name)
            for asset, source in zip(result["assets"], chosen):
                path = urllib.parse.quote(asset["path"], safe="")
                self.assertEqual(
                    self.call(f"/api/projects/{project['id']}/assets/{path}"),
                    source.read_bytes(),
                )
            with patch("studio.file_picker.choose", return_value=[chosen[0]]):
                duplicate = self.call(route, {"kind": "documents"})
            self.assertNotEqual(
                duplicate["assets"][0]["path"], result["assets"][0]["path"]
            )
        self.assertFalse(
            list(
                projects.assets_dir(studio.project_root(), project["id"]).glob("*.part")
            )
        )

    def test_native_picker_cancel_limit_and_validation(self):
        project = self.call("/api/projects", {"name": "Picker cancellation"})
        route = f"/api/projects/{project['id']}/assets/choose"
        with patch("studio.file_picker.choose", return_value=[]) as choose:
            self.assertEqual(
                self.call(route, {"kind": "media"}), {"assets": [], "errors": []}
            )
            choose.assert_called_once_with("media")
        with patch("studio.file_picker.choose") as choose:
            for body, headers, expected in (
                ({"kind": "unknown"}, {}, 422),
                ({"kind": "documents"}, {"X-Studio-Token": "invalid"}, 403),
            ):
                with self.assertRaises(urllib.error.HTTPError) as error:
                    self.call(route, body, headers=headers)
                self.assertEqual(error.exception.code, expected)
            choose.assert_not_called()
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "large.pdf"
            source.write_bytes(b"123456")
            with (
                patch("studio.file_picker.choose", return_value=[source]),
                patch("studio.MAX_ASSET_BYTES", 5),
            ):
                result = self.call(route, {"kind": "documents"})
            self.assertEqual(result["assets"], [])
            self.assertIn("smaller than", result["errors"][0]["message"])
            self.assertEqual(
                list(
                    projects.assets_dir(studio.project_root(), project["id"]).iterdir()
                ),
                [],
            )

    def test_native_picker_error_is_reported(self):
        project = self.call("/api/projects", {"name": "Picker busy"})
        with patch(
            "studio.file_picker.choose",
            side_effect=studio.file_picker.PickerError(
                "A file picker is already open."
            ),
        ):
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.call(
                    f"/api/projects/{project['id']}/assets/choose",
                    {"kind": "documents"},
                )
        self.assertEqual(error.exception.code, 409)
        self.assertIn("already open", error.exception.read().decode())

    def test_delete_last_project_resets_workspace_and_preserves_history(self):
        existing = self.call("/api/projects")["projects"]
        for project in existing[1:]:
            self.call(f"/api/projects/{project['id']}", method="DELETE")
        project = existing[0]
        old_folder = projects.folder(studio.project_root(), project["id"])
        history = studio.OUTPUTS / "kept.mp4"
        history.write_bytes(b"finished video")
        with (
            patch.object(studio, "ACTIVE", "active"),
            patch.dict(studio.JOBS, {"active": {"project": project["id"]}}),
        ):
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.call(f"/api/projects/{project['id']}", method="DELETE")
            self.assertEqual(error.exception.code, 409)
        result = self.call(f"/api/projects/{project['id']}", method="DELETE")
        replacement = result["replacement"]
        self.assertNotEqual(replacement["id"], project["id"])
        self.assertEqual(replacement["source"], "")
        self.assertFalse(old_folder.exists())
        self.assertEqual(history.read_bytes(), b"finished video")
        self.assertEqual(len(self.call("/api/projects")["projects"]), 1)

    def test_project_expansion_preference_persists(self):
        value = json.dumps({"a" * 32: False, "b" * 32: True})
        self.call("/api/preferences", {"values": {"manim-project-expanded": value}})
        self.assertEqual(self.call("/api/preferences")["manim-project-expanded"], value)

    def multipart(self, files):
        body = b""
        for name, data in files:
            body += (
                (
                    f'--studio-test\r\nContent-Disposition: form-data; name="files"; filename="{name}"\r\n'
                    "Content-Type: application/octet-stream\r\n\r\n"
                ).encode()
                + data
                + b"\r\n"
            )
        return body + b"--studio-test--\r\n"

    def test_folder_and_zip_import_endpoints(self):
        headers = {"Content-Type": "multipart/form-data; boundary=studio-test"}
        files = [
            ("Folder/script.py", SCENE.encode()),
            ("Folder/assets/nested/logo.svg", b"<svg/>"),
            ("Folder/narration.txt", b"[0:00] A circle."),
        ]
        folder = self.call(
            "/api/projects/import", raw=self.multipart(files), headers=headers
        )
        self.assertTrue(folder["narration"]["enabled"])
        self.assertEqual(self.call(f"/api/projects/{folder['id']}")["source"], SCENE)
        self.assertEqual(
            self.call(f"/api/projects/{folder['id']}/assets/nested/logo.svg"), b"<svg/>"
        )
        stream = io.BytesIO()
        with ZipFile(stream, "w") as archive:
            for name, data in files:
                archive.writestr(name, data)
        zipped = self.call(
            "/api/projects/import",
            raw=self.multipart([("Folder.zip", stream.getvalue())]),
            headers=headers,
        )
        self.assertNotEqual(folder["id"], zipped["id"])
        self.assertEqual(zipped["folder"], "Folder (2)")
        before = len(self.call("/api/projects")["projects"])
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.call(
                "/api/projects/import",
                raw=self.multipart([("bad.zip", b"broken")]),
                headers=headers,
            )
        self.assertEqual(error.exception.code, 409)
        self.assertEqual(len(self.call("/api/projects")["projects"]), before)
        with patch("projects.MAX_PACKAGE_BYTES", 10):
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.call(
                    "/api/projects/import", raw=self.multipart(files), headers=headers
                )
            self.assertIn(error.exception.code, (409, 422))
        with self.assertRaises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(
                urllib.request.Request(
                    self.url + "/api/projects/import",
                    data=self.multipart(files),
                    headers=headers,
                    method="POST",
                )
            )
        self.assertEqual(error.exception.code, 403)

    def test_asset_upload_list_serve_delete_and_traversal(self):
        project = self.call("/api/projects", {"name": "Assets"})
        base = f"/api/projects/{project['id']}/assets"
        uploaded = self.call(
            base,
            raw=b"\x89PNG fake",
            headers={"X-Filename": urllib.parse.quote("logo.png")},
        )
        self.assertEqual(uploaded, {"path": "logo.png", "size": 9, "kind": "image"})
        again = self.call(
            base, raw=b"two", headers={"X-Filename": urllib.parse.quote("logo.png")}
        )
        self.assertEqual(again["path"], "logo (2).png")
        self.assertEqual(
            [item["path"] for item in self.call(base)["assets"]],
            ["logo (2).png", "logo.png"],
        )
        self.assertEqual(self.call(base + "/logo.png"), b"\x89PNG fake")
        for bad in ("/..%2F..%2Fprojects", "/%2e%2e/%2e%2e/outputs"):
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.call(base + bad)
            self.assertEqual(error.exception.code, 404)
        self.call(base + "/logo.png", method="DELETE")
        self.assertEqual(len(self.call(base)["assets"]), 1)
        with self.assertRaises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(
                urllib.request.Request(
                    self.url + base,
                    data=b"x",
                    headers={"X-Filename": "a.png"},
                    method="POST",
                )
            )
        self.assertEqual(error.exception.code, 403)  # mutation token required

    def test_document_upload_bytes_duplicate_names_and_reload(self):
        project = self.call("/api/projects", {"name": "Document upload bytes"})
        base = f"/api/projects/{project['id']}/assets"
        for file in documents():
            uploaded = self.call(
                base,
                raw=file["buffer"],
                headers={"X-Filename": urllib.parse.quote(file["name"])},
            )
            self.assertEqual(
                uploaded,
                {"path": file["name"], "size": len(file["buffer"]), "kind": "file"},
            )
            self.assertEqual(
                self.call(base + "/" + urllib.parse.quote(uploaded["path"], safe="")),
                file["buffer"],
            )
        self.assertEqual(len(self.call(base)["assets"]), 4)
        zero = self.call(base, raw=b"", headers={"X-Filename": "empty.txt"})
        self.assertEqual(zero["size"], 0)
        self.assertEqual(self.call(base + "/empty.txt"), b"")
        with ThreadPoolExecutor(max_workers=4) as pool:
            uploads = list(
                pool.map(
                    lambda i: self.call(
                        base,
                        raw=f"Reference {i}".encode(),
                        headers={"X-Filename": "parallel.txt"},
                    ),
                    range(4),
                )
            )
        self.assertEqual(len({item["path"] for item in uploads}), 4)
        self.assertEqual(
            {
                self.call(base + "/" + urllib.parse.quote(item["path"], safe=""))
                for item in uploads
            },
            {f"Reference {i}".encode() for i in range(4)},
        )

    def test_document_upload_rejection_and_cancel_leave_no_partial_file(self):
        project = self.call("/api/projects", {"name": "Rejected document uploads"})
        base = f"/api/projects/{project['id']}/assets"
        directory = projects.assets_dir(studio.project_root(), project["id"])
        with self.assertRaises(urllib.error.HTTPError) as missing:
            self.call(base, raw=b"Reference", headers={"X-Filename": ""})
        self.assertEqual(missing.exception.code, 422)
        with patch("studio.MAX_ASSET_BYTES", 8):
            with self.assertRaises(urllib.error.HTTPError) as oversized:
                self.call(
                    base,
                    raw=b"Reference is too large",
                    headers={"X-Filename": "notes.txt"},
                )
            self.assertEqual(oversized.exception.code, 413)
        self.assertEqual(list(directory.iterdir()), [])

        class InterruptedUpload:
            headers = {"X-Filename": "notes.txt"}

            async def stream(self):
                yield b"Partial reference"
                raise ClientDisconnect()

        with self.assertRaises(ClientDisconnect):
            asyncio.run(studio.upload_project_asset(project["id"], InterruptedUpload()))
        self.assertEqual(list(directory.iterdir()), [])
        retried = self.call(
            base, raw=b"Complete reference", headers={"X-Filename": "notes.txt"}
        )
        self.assertEqual(retried["path"], "notes.txt")

    def test_project_render_resolves_its_own_assets(self):
        project = self.call("/api/projects", {"name": "Uses assets"})
        with tempfile.TemporaryDirectory() as folder:
            image = Path(folder) / "dot.png"
            av_image = np.zeros((8, 8, 3), dtype=np.uint8)
            av_image[2:6, 2:6] = 255
            from PIL import Image

            Image.fromarray(av_image).save(image)
            self.call(
                f"/api/projects/{project['id']}/assets",
                raw=image.read_bytes(),
                headers={"X-Filename": "dot.png"},
            )
        source = (
            "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n"
            '        self.add(ImageMobject("assets/dot.png"))\n        self.wait(0.2)\n'
        )
        job = self.call(
            "/api/render",
            {
                "source": source,
                "scene": "MainScene",
                "project": project["id"],
                "width": 128,
                "height": 72,
                "fps": 10,
            },
        )
        result = self.wait(job["id"])
        self.assertEqual(result["status"], "completed", result.get("log"))
        self.assertEqual(result["project"], project["id"])
        self.assertEqual(result["project_name"], "Uses assets")
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.call(
                "/api/render",
                {"source": source, "scene": "MainScene", "project": "f" * 32},
            )
        self.assertEqual(error.exception.code, 404)

    def test_narration_requires_the_voice_model(self):
        if narration.installed(studio.ROOT, "high"):
            self.skipTest("High quality model is installed")
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.call(
                "/api/render",
                {
                    "source": SCENE,
                    "scene": "MainScene",
                    "narration": {"enabled": True, "text": "Hello", "quality": "high"},
                },
            )
        self.assertEqual(error.exception.code, 422)
        self.assertIn("Download the narration voice", error.exception.read().decode())
        # Enabled narration with no spoken lines renders normally.
        job = self.call(
            "/api/render",
            {
                "source": SCENE,
                "scene": "MainScene",
                "width": 128,
                "height": 72,
                "fps": 10,
                "narration": {
                    "enabled": True,
                    "text": "# only a note",
                    "quality": "high",
                },
            },
        )
        self.assertEqual(self.wait(job["id"])["status"], "completed")
        status = self.call("/api/narration")
        self.assertFalse(status["models"]["high"]["installed"])
        self.assertIn("af_heart", [voice["id"] for voice in status["voices"]])

    @unittest.skipUnless(
        os.environ.get("MANIM_STUDIO_TEST_KOKORO"),
        "Set MANIM_STUDIO_TEST_KOKORO to a folder with the Kokoro model files",
    )
    def test_narrated_render_and_voice_preview(self):
        wav = self.call(
            "/api/narration/preview",
            {"text": "Testing one two.", "voice": "bf_emma", "quality": "standard"},
        )
        self.assertTrue(wav.startswith(b"RIFF"))
        job = self.call(
            "/api/render",
            {
                "source": SCENE,
                "scene": "MainScene",
                "width": 128,
                "height": 72,
                "fps": 10,
                "narration": {
                    "enabled": True,
                    "text": "[0:00] A circle.\n\n[0:00.2] It overlaps.",
                    "voice": "af_heart",
                },
            },
        )
        result = self.wait(job["id"])
        self.assertEqual(result["status"], "completed", result.get("log"))
        self.assertTrue(result["narrated"])
        self.assertTrue(
            any("overlap" in w or "past the animation" in w for w in result["warnings"])
        )
        path = studio.output_directory(result) / "video.mp4"
        with av.open(str(path)) as video:
            self.assertTrue(video.streams.audio)
        manifest = json.loads((path.parent / "render.json").read_text())
        self.assertTrue(manifest["narrated"])
        self.assertNotIn("_narration", manifest)


if __name__ == "__main__":
    unittest.main()
