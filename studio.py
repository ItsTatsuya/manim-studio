"""Local-only Manim Studio server. Scripts run in a separate Python process."""

from __future__ import annotations

import asyncio
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import importlib.metadata
import importlib.util
import io
import itertools
import json
import logging
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import threading
import time
import uuid
import tokenize
from urllib.parse import unquote

from fastapi import FastAPI, HTTPException, Request
from fastapi.exception_handlers import request_validation_exception_handler
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.trustedhost import TrustedHostMiddleware
from pydantic import BaseModel, Field, PrivateAttr, field_validator, model_validator
from starlette.concurrency import run_in_threadpool
from starlette.datastructures import UploadFile
from starlette.formparsers import MultiPartException, MultiPartParser
from runtime import (
    APP_ROOT,
    APP_VERSION,
    DATA_ROOT,
    PACKAGE_ROOT,
    PORTABLE,
    STANDALONE,
    renderer_python,
)
from updates import UpdateManager, UpdateError, UpdateBusy
from processes import (
    attach_process_tree,
    popen_options,
    release_process_tree,
    start_process_monitor,
    stop_process,
)
import agents
import agent_updates
import file_picker
from events import BUS, publish
import narration
import usage
import projects
import watermark
import scenes
import styles
import storyboard
from scenes import format_source
from video_workflow import VideoProblem, VideoWorkflows

WatermarkSettings = watermark.Settings

ROOT = DATA_ROOT
WEB = APP_ROOT / "studio_ui"
RENDERS = ROOT / ".renders" / "studio"
OUTPUTS = ROOT / "outputs" / "studio"
TOKEN = secrets.token_urlsafe(32)
QUALITY = {
    "preview": ("-ql", "480p", 15),
    "standard": ("-qm", "720p", 30),
    # 30 fps is plenty for explainers and halves the render time of 60.
    "final": ("-qh", "1080p", 30),
}
# Manim's own frame rate for each quality flag; a preset that differs says so.
FLAG_FPS = {"-ql": 15, "-qm": 30, "-qh": 60}
QUALITY_DIMENSIONS = {
    "preview": (854, 480),
    "standard": (1280, 720),
    "final": (1920, 1080),
}
MAX_CAIRO_DIMENSION = 32766
LOCK = threading.RLock()
JOBS: dict[str, dict] = {}
ACTIVE: str | None = None
# None follows the "start" preference; True holds waiting videos; False runs
# the queue until it drains, then returns to the preference.
QUEUE_PAUSED: bool | None = None
QUEUE_SEQ = itertools.count(1)
PROCESS: subprocess.Popen | None = None
WORKER: threading.Thread | None = None
ANSI = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]")
STUDIO_MARKER = re.compile(
    r"^STUDIO_(?:STAGE|WARNING|NARRATION_ERROR|WATERMARK_ERROR)\|"
)
UPDATES = UpdateManager(
    APP_VERSION,
    "portable" if PORTABLE else "installed" if STANDALONE else "source",
    PACKAGE_ROOT,
    DATA_ROOT,
)
UPDATE_INSTALLING = False
UPDATE_HANDLER = None
AGENTS = agents.AgentManager(
    DATA_ROOT,
    python=renderer_python(),
    app_root=APP_ROOT,
    latex=lambda: shutil.which("latex") and shutil.which("dvisvgm"),
)
NARRATION = narration.ModelDownloader(DATA_ROOT)
# desktop.py turns on the automatic check; tests and imports stay manual.
AGENT_UPDATES = agent_updates.AgentUpdater(lambda: AGENTS, automatic=False)
MAX_ASSET_BYTES = 2 * 1024**3


def frame_rate_fraction(value: float) -> Fraction:
    rate = Fraction(str(value))
    if rate.numerator > 2**31 - 1 or rate.denominator > 2**31 - 1:
        rate = rate.limit_denominator(1_000_000)
    return rate


class Script(BaseModel):
    source: str = Field(min_length=1, max_length=500_000)


class NarrationSettings(BaseModel):
    enabled: bool = False
    text: str = Field(default="", max_length=100_000)
    voice: str = Field(default="af_heart", max_length=40)
    speed: float = Field(default=1.0, ge=0.5, le=2.0, allow_inf_nan=False)
    quality: str = "high"


class RenderRequest(Script):
    _workflow: str | None = PrivateAttr(default=None)
    queue: bool = False
    workspace: str | None = Field(default=None, max_length=32)
    scene: str = Field(min_length=1, max_length=200)
    project: str | None = Field(default=None, max_length=32)
    narration: NarrationSettings | None = None
    # Explicit watermark; when omitted the project's (or global) settings apply.
    watermark: WatermarkSettings | None = None
    quality: str = "preview"
    name: str = Field(default="Untitled animation", max_length=100)
    width: int | None = Field(default=None, strict=True, gt=0, le=MAX_CAIRO_DIMENSION)
    height: int | None = Field(default=None, strict=True, gt=0, le=MAX_CAIRO_DIMENSION)
    fps: float | None = Field(default=None, strict=True, gt=0, allow_inf_nan=False)

    @field_validator("width", "height")
    @classmethod
    def even_dimension(cls, value):
        if value is not None and value % 2:
            raise ValueError("MP4 width and height must be even pixel counts.")
        return value

    @field_validator("fps")
    @classmethod
    def representable_frame_rate(cls, value):
        if value is not None:
            rate = frame_rate_fraction(value)
            if not 0 < rate.numerator <= 2**31 - 1 or rate.denominator > 2**31 - 1:
                raise ValueError(
                    "Frame rate cannot be represented by the video encoder."
                )
            if not math.isclose(float(rate), value, rel_tol=1e-9, abs_tol=0):
                raise ValueError("Frame rate is too precise for the video encoder.")
            # MP4 doubles the track timescale from the rate numerator to >=10000.
            # Its resulting frame duration must fit the muxer's signed integer.
            scale = 1
            while rate.numerator * scale < 10_000:
                scale *= 2
            if rate.denominator * scale > 2**31 - 1:
                raise ValueError("Frame rate is too low for MP4 frame timing.")
            return float(rate)
        return value

    @model_validator(mode="after")
    def paired_dimensions(self):
        if (self.width is None) != (self.height is None):
            raise ValueError("Set both width and height for a custom resolution.")
        return self


class Preferences(BaseModel):
    values: dict[str, str]


PREFERENCE_KEYS = {
    # Earlier single-script releases; read once to create the first project.
    "manim-draft",
    "manim-name",
    "manim-quality",
    "manim-resolution",
    "manim-fps",
    "manim-theme",
    "manim-check-updates",
    "manim-project",
    "manim-project-expanded",
    "studio-layout",
    "studio-sidebar-collapsed",
    "studio-threads",
    "studio-onboarding",
}


def valid_unicode(value: str) -> bool:
    try:
        value.encode("utf-8")
        return True
    except UnicodeEncodeError:
        return False


def preferences():
    path = ROOT / ".studio" / "preferences.json"
    with LOCK:
        try:
            saved = json.loads(path.read_text(encoding="utf-8"))
            return (
                {
                    key: value
                    for key, value in saved.items()
                    if key in PREFERENCE_KEYS
                    and isinstance(value, str)
                    and len(value) <= 500_000
                    and valid_unicode(value)
                }
                if isinstance(saved, dict)
                else {}
            )
        except (OSError, ValueError, RecursionError):
            return {}


def save_preferences(body: Preferences):
    if any(not valid_unicode(value) for value in body.values.values()):
        raise HTTPException(
            422,
            "This request contains invalid Unicode text. Paste or import valid text again.",
        )
    if any(
        key not in PREFERENCE_KEYS or len(value) > 500_000
        for key, value in body.values.items()
    ):
        raise HTTPException(422, "Invalid preference.")
    with LOCK:
        saved = preferences()
        saved.update(body.values)
        directory = ROOT / ".studio"
        directory.mkdir(parents=True, exist_ok=True)
        temporary = directory / "preferences.tmp"
        temporary.write_text(json.dumps(saved), encoding="utf-8")
        temporary.replace(directory / "preferences.json")
    return {"ok": True}


def analyze(source: str) -> dict:
    try:
        return scenes.analyze(source)
    except scenes.ScriptError as error:
        raise HTTPException(422, str(error)) from error


def public_job(job: dict) -> dict:
    return {key: value for key, value in job.items() if not key.startswith("_")}


def job_snapshot(job: dict) -> dict:
    result = public_job(job)
    # A terminal outcome is retry-ready only after process and media cleanup.
    if ACTIVE == job["id"] and job["status"] in {"completed", "failed", "cancelled"}:
        result.update(
            status="rendering", stage="Finishing render cleanup…", can_cancel=False
        )
    return result


def valid_manifest(job):
    """Ignore incomplete or damaged history without preventing app startup."""
    if not isinstance(job, dict):
        return False
    strings = (
        "id",
        "name",
        "scene",
        "quality",
        "created",
        "status",
        "resolution",
        "video",
    )
    if any(
        not isinstance(job.get(key), str) or not valid_unicode(job[key])
        for key in strings
    ):
        return False
    if (
        not re.fullmatch(r"[0-9a-f]{32}", job["id"])
        or job["quality"] not in QUALITY
        or job["status"] != "completed"
    ):
        return False
    if "width" in job or "height" in job:
        if any(
            not isinstance(job.get(key), int)
            or isinstance(job[key], bool)
            or not 0 < job[key] <= MAX_CAIRO_DIMENSION
            or job[key] % 2
            for key in ("width", "height")
        ):
            return False
    if any(
        not isinstance(job.get(key), (int, float))
        or isinstance(job[key], bool)
        or (isinstance(job[key], float) and not math.isfinite(job[key]))
        or job[key] < 0
        or job[key] > 10**15
        for key in ("size", "fps", "elapsed")
    ):
        return False
    try:
        datetime.fromisoformat(job["created"])
        # Optional legacy metadata must also be safe to return as JSON.
        json.dumps(job, allow_nan=False, ensure_ascii=False).encode("utf-8")
    except (ValueError, UnicodeEncodeError, RecursionError):
        return False
    return True


def safe_folder_name(name: str) -> str:
    """Keep the title readable while respecting Windows folder-name rules."""
    return projects.safe_name(name, "Untitled animation", limit=200)


def reserve_output_folder(name: str) -> str:
    """Atomically reserve a title-based folder; never overwrite another render."""
    OUTPUTS.mkdir(parents=True, exist_ok=True)
    existing = {path.name.casefold() for path in OUTPUTS.iterdir()}
    stem = safe_folder_name(name)
    number = 1
    while True:
        folder = stem if number == 1 else f"{stem} ({number})"
        if folder.casefold() not in existing:
            try:
                (OUTPUTS / folder).mkdir()
                return folder
            except FileExistsError:
                existing.add(folder.casefold())
        number += 1


def output_directory(job: dict) -> Path:
    path = OUTPUTS / job.get("folder", job["id"])
    try:
        contained = path.resolve().parent == OUTPUTS.resolve()
    except (OSError, RuntimeError):
        contained = False
    if not contained:
        raise HTTPException(404, "Render folder not found.")
    return path


def output_file(job: dict, filename: str) -> Path:
    directory = output_directory(job).resolve()
    path = directory / filename
    try:
        contained = path.resolve().parent == directory
    except (OSError, RuntimeError):
        contained = False
    if not contained:
        raise HTTPException(404, "File not found.")
    return path


def cancel_active():
    with LOCK:
        if ACTIVE and ACTIVE in JOBS:
            JOBS[ACTIVE]["_cancel"] = True
        process = PROCESS
    stop_process(process)


def shutdown_render():
    with LOCK:
        for job in JOBS.values():
            if job["status"] == "queued" and job["id"] != ACTIVE:
                job.update(status="cancelled", stage="Cancelled on shutdown")
                job.pop("_source", None)
    try:
        try:
            cancel_active()
        except (OSError, subprocess.SubprocessError):
            logging.exception("Unable to cancel the render during shutdown")
            with LOCK:
                process = PROCESS
            try:
                if process is not None and process.poll() is None:
                    process.kill()
            except (OSError, subprocess.SubprocessError):
                logging.exception("Unable to kill the render process during shutdown")
    finally:
        with LOCK:
            worker = WORKER
        if worker is not None and worker is not threading.current_thread():
            worker.join(timeout=15)
            if worker.is_alive():
                logging.error(
                    "Render cleanup did not finish within the shutdown deadline"
                )


@asynccontextmanager
async def lifespan(app):
    OUTPUTS.mkdir(parents=True, exist_ok=True)
    NARRATION.data_root = ROOT
    try:
        projects.ensure_default(ROOT / "projects", ROOT / "assets", preferences())
    except OSError:
        logging.exception("Unable to prepare the first project")
    for manifest in OUTPUTS.glob("*/render.json"):
        try:
            job = json.loads(manifest.read_text(encoding="utf-8"))
            if (
                valid_manifest(job)
                and (manifest.parent / "video.mp4").is_file()
                and (manifest.parent / "video.mp4").stat().st_size > 0
                and (manifest.parent / "video.mp4").resolve().parent
                == manifest.parent.resolve()
                and not manifest.parent.is_symlink()
                and manifest.parent.resolve().parent == OUTPUTS.resolve()
            ):
                job = public_job(job)
                job["folder"] = manifest.parent.name
                job.pop("log", None)
                job["video"] = f"/api/renders/{job['id']}/video"
                JOBS[job["id"]] = job
        except (ValueError, KeyError, OSError, RuntimeError):
            continue
    AGENTS.data_root = ROOT
    VIDEOS.closed = False
    reset_queue()
    BUS.open()
    AGENT_UPDATES.load()
    AGENT_UPDATES.start()
    watcher_stop = threading.Event()
    watcher = threading.Thread(target=watch_renders, args=(watcher_stop,), daemon=True)
    watcher.start()
    try:
        yield
    finally:
        watcher_stop.set()
        VIDEOS.close()
        BUS.close()
        await asyncio.to_thread(AGENT_UPDATES.shutdown)
        await asyncio.to_thread(AGENTS.shutdown)
        await asyncio.to_thread(shutdown_render)
        await asyncio.to_thread(UPDATES.shutdown)
        await asyncio.to_thread(NARRATION.shutdown)
        await asyncio.to_thread(watcher.join, 2)


def render_summary(job: dict) -> dict:
    return {key: value for key, value in job_snapshot(job).items() if key != "log"}


def watch_renders(stop: threading.Event):
    """Push the active render's progress to every window as it changes."""
    watched, last = None, None
    while not stop.wait(0.25):
        with LOCK:
            current = ACTIVE or watched
            job = JOBS.get(current) if current else None
            summary = render_summary(job) if job else None
            watched = ACTIVE
            finished = [
                render_summary(item)
                for item in JOBS.values()
                if item.get("workflow")
                and item["status"] in {"completed", "failed", "cancelled"}
                and item["id"] != ACTIVE
                and not item.get("_video_notified")
            ]
            for item in finished:
                JOBS[item["id"]]["_video_notified"] = True
        for item in finished:
            VIDEOS.render_finished(item)
        if summary is None:
            continue
        key = json.dumps(summary, sort_keys=True, default=str)
        if key != last:
            last = key
            publish("render", {"render": summary})


app = FastAPI(lifespan=lifespan, docs_url=None, redoc_url=None, openapi_url=None)


@app.exception_handler(RequestValidationError)
async def invalid_request(request: Request, error: RequestValidationError):
    try:
        return await request_validation_exception_handler(request, error)
    except (UnicodeError, ValueError, RecursionError):
        # Invalid nested input can also make the normal error echo unserializable.
        return JSONResponse(
            {
                "detail": "This request contains invalid text or numbers. Check the input and try again."
            },
            status_code=422,
        )


app.add_middleware(
    TrustedHostMiddleware, allowed_hosts=["127.0.0.1", "localhost", "testserver"]
)
app.get("/api/preferences")(preferences)
app.post("/api/preferences")(save_preferences)


@app.middleware("http")
async def local_only(request: Request, call_next):
    if request.method not in {"GET", "HEAD"} and not secrets.compare_digest(
        request.headers.get("X-Studio-Token", "").encode("utf-8"), TOKEN.encode("ascii")
    ):
        return HTMLResponse(
            "This request must come from your Studio window.", status_code=403
        )
    response = await call_next(request)
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["Referrer-Policy"] = "no-referrer"
    response.headers["X-Frame-Options"] = "DENY"
    if request.url.path == "/" or request.url.path.startswith("/api"):
        response.headers["Cache-Control"] = "no-store"
    return response


@app.get("/", response_class=HTMLResponse)
def index():
    try:
        page = (WEB / "index.html").read_text(encoding="utf-8")
    except OSError:
        page = (
            '<!doctype html><meta name="studio-token" content="__STUDIO_TOKEN__">'
            "<p>The Studio interface has not been built. Run npm run build.</p>"
        )
    return page.replace("__STUDIO_TOKEN__", TOKEN)


@app.get("/api/events")
async def event_stream():
    """One long-lived stream of render, agent and project changes."""
    from fastapi.responses import StreamingResponse

    return StreamingResponse(
        BUS.stream({"version": APP_VERSION}),
        media_type="text/event-stream",
        headers={"X-Accel-Buffering": "no"},
    )


_TOOLS = {"checked": 0.0, "latex": False, "typst": False, "manim": ""}
TOOLS_TTL = 30.0


def installed_tools() -> dict:
    """Probe the PATH and package metadata at most every TOOLS_TTL seconds.

    These lookups scan the disk (several milliseconds), and /api/health runs on
    every page load and render start.
    """
    now = time.monotonic()
    with LOCK:
        if now - _TOOLS["checked"] >= TOOLS_TTL or not _TOOLS["manim"]:
            _TOOLS.update(
                checked=now,
                manim=_TOOLS["manim"] or importlib.metadata.version("manim"),
                latex=bool(shutil.which("latex") and shutil.which("dvisvgm")),
                typst=importlib.util.find_spec("typst") is not None,
            )
        return dict(_TOOLS)


@app.get("/api/health")
def health():
    tools = installed_tools()
    return {
        "manim": tools["manim"],
        "python": sys.version.split()[0],
        "latex": tools["latex"],
        "typst": tools["typst"],
        "active": ACTIVE,
        "assets": str(ROOT / "assets"),
        "outputs": str(OUTPUTS),
        "standalone": STANDALONE,
        "native_file_picker": file_picker.AVAILABLE,
        "video_workspaces": True,
        "portable": PORTABLE,
        "data": str(DATA_ROOT),
        "version": APP_VERSION,
    }


def configure_update_install(handler):
    global UPDATE_HANDLER
    with LOCK:
        UPDATE_HANDLER = handler
        UPDATES.set_install_supported(handler is not None)


def update_install_failed(message):
    global UPDATE_INSTALLING
    with LOCK:
        UPDATE_INSTALLING = False
        UPDATES.install_failed(message)


def update_action(action):
    try:
        return action()
    except UpdateBusy as error:
        raise HTTPException(409, str(error)) from error
    except UpdateError as error:
        raise HTTPException(422, str(error)) from error


@app.get("/api/updates")
def update_status():
    return UPDATES.snapshot()


@app.post("/api/updates/check")
def check_update():
    return update_action(UPDATES.check)


@app.post("/api/updates/download")
def download_update():
    return update_action(UPDATES.download)


@app.post("/api/updates/cancel")
def cancel_update():
    return update_action(UPDATES.cancel)


@app.post("/api/updates/install")
def install_update():
    global UPDATE_INSTALLING
    with LOCK:
        if ACTIVE or UPDATE_INSTALLING:
            raise HTTPException(
                409, "Finish the current render or update before installing."
            )
        if UPDATE_HANDLER is None:
            raise HTTPException(
                409, "Open the packaged desktop app to install updates."
            )
        handoff = update_action(UPDATES.begin_install)
        UPDATE_INSTALLING = True
        handler = UPDATE_HANDLER
    try:
        handler(handoff)
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        logging.exception("Unable to prepare the update installer")
        update_install_failed(str(error))
        raise HTTPException(
            503, "The update could not start. Studio is still open. Try again."
        ) from error
    return UPDATES.snapshot()


@app.post("/api/analyze")
def analyze_script(body: Script):
    return analyze(body.source)


@app.post("/api/format")
def format_script(body: Script):
    try:
        return {"source": format_source(body.source)}
    except (
        SyntaxError,
        tokenize.TokenError,
        IndentationError,
        RecursionError,
        UnicodeEncodeError,
    ) as error:
        raise HTTPException(
            422, "Fix Python syntax errors before formatting."
        ) from error


@app.get("/api/renders")
def list_renders():
    with LOCK:
        # History needs metadata; full logs are fetched only for the active job.
        return [
            {key: value for key, value in job_snapshot(job).items() if key != "log"}
            for job in sorted(
                JOBS.values(), key=lambda job: job["created"], reverse=True
            )
        ]


def video_settings(path: Path) -> dict:
    import av

    with av.open(str(path)) as video:
        if not video.streams.video:
            raise RuntimeError("The rendered file does not contain a video stream.")
        stream = video.streams.video[0]
        if stream.average_rate is None or stream.average_rate <= 0:
            raise RuntimeError(
                "The rendered video does not contain a valid frame rate."
            )
        return {
            "width": stream.width,
            "height": stream.height,
            "resolution": f"{stream.width}x{stream.height}",
            "fps": float(stream.average_rate),
        }


def output_quality(width: int, height: int, fps: float) -> str:
    return next(
        (
            quality
            for quality, dimensions in QUALITY_DIMENSIONS.items()
            if (width, height, fps) == (*dimensions, QUALITY[quality][2])
        ),
        "standard",
    )


def video_cache_key(job: dict, source: str, working: Path) -> str:
    """Identify an animation by everything that can change its frames."""
    parts = [
        "1",
        source,
        job["scene"],
        job["quality"],
        job.get("width"),
        job.get("height"),
        job.get("fps"),
        job.get("_requested_rate"),
        job.get("project"),
    ]
    spoken = job.get("_narration")
    if spoken and narration.uses_timing(source):
        # narrate() blocks last as long as their lines, so the voice shapes frames.
        parts.append(
            [narration.CACHE_VERSION]
            + [spoken.get(key) for key in ("text", "voice", "speed", "quality")]
        )
    files = []
    if job.get("project"):
        # Assets and helper modules beside the script affect the render too.
        # Compiled caches change on every run and must not invalidate it.
        try:
            for current, folders, names in os.walk(working):
                folders[:] = sorted(
                    name
                    for name in folders
                    if name != "__pycache__" and not name.startswith(".")
                )
                for name in sorted(names):
                    # The storyboard is a plan for review; it never changes frames.
                    if name in {
                        "script.py",
                        "narration.txt",
                        "project.json",
                        storyboard.FILE,
                        storyboard.STATE,
                    }:
                        continue
                    path = Path(current) / name
                    info = path.stat()
                    files.append(
                        [
                            path.relative_to(working).as_posix(),
                            info.st_size,
                            info.st_mtime_ns,
                        ]
                    )
                    if len(files) >= 5000:
                        raise StopIteration
        except StopIteration:
            pass
        except OSError:
            files.append("unreadable")
    parts.append(files)
    return hashlib.sha256(json.dumps(parts, default=str).encode("utf-8")).hexdigest()[
        :32
    ]


def render_worker(job_id: str, source: str):
    global ACTIVE, PROCESS, WORKER
    job = JOBS[job_id]
    directory = RENDERS / job_id
    started = time.monotonic()
    process = None
    monitor = None
    staged_files = []
    try:
        destination = output_directory(job)
        directory.mkdir(parents=True, exist_ok=True)
        destination.mkdir(parents=True, exist_ok=True)
        script = directory / "animation.py"
        script.write_text(source, encoding="utf-8")
        (destination / "script.py").write_text(source, encoding="utf-8")
        flag, resolution, fps = QUALITY[job["quality"]]
        # Project renders run inside their folder so assets/... paths resolve there.
        working = ROOT
        if job.get("project"):
            working = projects.workspace_folder(
                ROOT / "projects", job["project"], job.get("workspace")
            )
            (working / projects.ASSETS).mkdir(exist_ok=True)
            try:
                # Refresh the house-style helper before the cache key reads it.
                styles.write_module(ROOT, working)
            except OSError:
                logging.exception("Unable to write the project style module")
            try:
                narration.write_module(working)
            except OSError:
                logging.exception("Unable to write the narration timing module")
        config = ROOT / "manim.cfg"
        render_options = (
            ["--config_file", str(config)]
            if working != ROOT and config.is_file()
            else []
        )
        if "width" not in job and FLAG_FPS.get(flag) != fps:
            render_options += ["--fps", str(fps)]
        if "width" in job:
            resolution = f"{job['width']}x{job['height']}"
            fps = job["fps"]
            render_options += [
                "--resolution",
                f"{job['width']},{job['height']}",
                "--fps",
                str(job.get("_requested_fps", fps)),
            ]
        command = [
            renderer_python(),
            "-u",
            str(APP_ROOT / "worker.py"),
            flag,
            *render_options,
            "--renderer",
            "cairo",
            "--format",
            "mp4",
            "--progress_bar",
            "display",
            # Skip Manim's per-render PyPI version check (up to 10 s offline).
            "--silent",
            "--media_dir",
            str(directory / "media"),
            "-o",
            "video",
            str(script),
            job["scene"],
        ]
        env = dict(
            os.environ,
            PYTHONUNBUFFERED="1",
            PYTHONIOENCODING="utf-8",
            MANIM_STUDIO_WORKER_GATE="1",
            # Helpers must build identical scenes, so set iteration order is pinned.
            PYTHONHASHSEED="0",
            PYTHONPATH=str(ROOT) + os.pathsep + os.environ.get("PYTHONPATH", ""),
        )
        env.pop("MANIM_STUDIO_CUSTOM_FPS", None)
        env.pop("MANIM_STUDIO_NARRATION", None)
        env.pop("MANIM_STUDIO_WATERMARK", None)
        env.pop("MANIM_STUDIO_PARALLEL", None)
        env.pop("MANIM_STUDIO_HELPER", None)
        # Long scenes are split across helper processes (parallel.py); setting
        # MANIM_STUDIO_RENDER_WORKERS to 1 keeps every render in one process.
        workers = os.environ.get("MANIM_STUDIO_RENDER_WORKERS", "auto")
        if workers not in {"0", "1", "off"}:
            env["MANIM_STUDIO_PARALLEL"] = workers
        if "_requested_fps" in job:
            env["MANIM_STUDIO_CUSTOM_FPS"] = job["_requested_rate"]
        if job.get("_narration"):
            spec = directory / "narration.json"
            spec.write_text(
                json.dumps(
                    {
                        **job["_narration"],
                        "media_dir": str(directory / "media"),
                        "data_root": str(ROOT),
                        # Checkpoints outlive this job so a retry resumes.
                        "line_cache": str(ROOT / ".studio" / "render-cache" / "lines"),
                        "video_cache": str(
                            ROOT
                            / ".studio"
                            / "render-cache"
                            / "videos"
                            / video_cache_key(job, source, working)
                        ),
                    }
                ),
                encoding="utf-8",
            )
            env["MANIM_STUDIO_NARRATION"] = str(spec)
        if job.get("_watermark"):
            # Applied after narration, so the cached animation stays unmarked.
            spec = directory / "watermark.json"
            spec.write_text(
                json.dumps(
                    {**job["_watermark"], "media_dir": str(directory / "media")}
                ),
                encoding="utf-8",
            )
            env["MANIM_STUDIO_WATERMARK"] = str(spec)
        with LOCK:
            if job.get("_cancel"):
                job.update(status="cancelled", stage="Render cancelled")
                return
            PROCESS = subprocess.Popen(
                command,
                cwd=working,
                env=env,
                stdout=subprocess.PIPE,
                stdin=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                **popen_options(),
            )
            process = PROCESS
            attach_process_tree(process)
            monitor = start_process_monitor(process)
            process.stdin.write("1\n")
            process.stdin.flush()
            process.stdin.close()
            job.update(status="rendering", stage="Starting Manim…")
        with (destination / "render.log").open("w", encoding="utf-8") as log:
            for line in process.stdout:
                clean = ANSI.sub("", line).strip()
                readable = STUDIO_MARKER.sub("", clean)
                log.write(readable + "\n")
                log.flush()
                with LOCK:
                    job["log"] = (job["log"] + readable + "\n")[-40_000:]
                    if clean.startswith("STUDIO_ERROR|"):
                        try:
                            problem = json.loads(clean.split("|", 1)[1])
                            if isinstance(problem.get("line"), int) and 1 <= problem[
                                "line"
                            ] <= len(source.splitlines()):
                                job["problem"] = {
                                    "line": problem["line"],
                                    "message": str(problem["message"])[:4000],
                                }
                        except (
                            ValueError,
                            KeyError,
                            TypeError,
                            AttributeError,
                            RecursionError,
                        ):
                            pass
                    match = re.search(r"Animation (\d+).*?(\d+)%", clean)
                    if clean.startswith("STUDIO_STAGE|"):
                        job["stage"] = clean.split("|", 1)[1][:200]
                    elif clean.startswith("STUDIO_WARNING|"):
                        job.setdefault("warnings", []).append(
                            clean.split("|", 1)[1][:1000]
                        )
                    elif clean.startswith("STUDIO_NARRATION_ERROR|"):
                        job["_narration_error"] = clean.split("|", 1)[1][:1000]
                    elif clean.startswith("STUDIO_WATERMARK_ERROR|"):
                        job["_watermark_error"] = clean.split("|", 1)[1][:1000]
                    elif match:
                        job["stage"] = f"Animation {int(match[1]) + 1} · {match[2]}%"
                    elif "Combining" in clean or "Writing" in clean:
                        job["stage"] = "Putting your video together…"
                    job["elapsed"] = round(time.monotonic() - started, 1)
            result = process.wait()
        with LOCK:
            if job.get("_cancel"):
                job.update(status="cancelled", stage="Render cancelled")
                return
        if result and job.get("_narration_error"):
            raise RuntimeError(job["_narration_error"])
        if result and job.get("_watermark_error"):
            raise RuntimeError(job["_watermark_error"])
        if result:
            job["repairable"] = True
            raise RuntimeError(
                "Manim couldn't render this script. The render log contains the error details."
            )
        matches = [
            path
            for path in directory.rglob("video.mp4")
            if "partial_movie_files" not in path.parts
        ]
        if not matches:
            job["repairable"] = True
            raise RuntimeError(
                "The scene didn't produce a video. Add an animation with self.play() or a pause with self.wait(), then render again."
            )
        video = destination / "video.tmp"
        staged_files.append(video)
        shutil.copy2(
            max(matches, key=lambda path: path.stat().st_mtime_ns),
            video,
        )
        actual_settings = video_settings(video) if "width" in job else {}
        if actual_settings:
            resolution, fps = actual_settings["resolution"], actual_settings["fps"]
            actual_settings["quality"] = output_quality(
                actual_settings["width"], actual_settings["height"], fps
            )
        shortcut = None
        shortcut_warning = False
        preset_width, preset_height = QUALITY_DIMENSIONS[job["quality"]]
        preset_fps = QUALITY[job["quality"]][2]
        if (
            job["quality"] in {"preview", "final"}
            and (
                job.get("width", preset_width),
                job.get("height", preset_height),
                job.get("_requested_fps", job.get("fps", preset_fps)),
            )
            == (preset_width, preset_height, preset_fps)
            and (
                actual_settings.get("width", preset_width),
                actual_settings.get("height", preset_height),
                fps,
            )
            == (preset_width, preset_height, preset_fps)
        ):
            shortcut = ROOT / "outputs" / f".{job['quality']}-{job_id}.tmp"
            staged_files.append(shortcut)
            try:
                shutil.copy2(video, shortcut)
            except OSError:
                shortcut_warning = True
                shortcut = None
        completed = dict(
            status="completed",
            stage="Your video is ready",
            elapsed=round(time.monotonic() - started, 1),
            size=video.stat().st_size,
            video=f"/api/renders/{job_id}/video",
            resolution=resolution,
            fps=fps,
        )
        completed.update(actual_settings)
        manifest = {
            key: value
            for key, value in {**public_job(job), **completed}.items()
            if key != "log"
        }
        temporary = destination / "render.tmp"
        staged_files.append(temporary)
        temporary.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
        with LOCK:
            if job.get("_cancel"):
                job.update(status="cancelled", stage="Render cancelled")
                return
            # Completion and cancellation share one commit boundary.
            video.replace(destination / "video.mp4")
            try:
                temporary.replace(destination / "render.json")
            except OSError:
                (destination / "video.mp4").unlink(missing_ok=True)
                raise
            job.update(completed)
            if shortcut is not None:
                try:
                    shortcut.replace(ROOT / "outputs" / f"{job['quality']}.mp4")
                except OSError:
                    shortcut_warning = True
        if shortcut_warning:
            try:
                with (destination / "render.log").open("a", encoding="utf-8") as log:
                    log.write(
                        "\nThe video is saved in Studio history. Close any player using the previous shortcut video to update it.\n"
                    )
            except OSError:
                logging.exception("Unable to append the shortcut warning to render.log")
    except Exception as error:
        with LOCK:
            job.update(
                status="cancelled" if job.get("_cancel") else "failed",
                error=str(error),
                stage="Render cancelled"
                if job.get("_cancel")
                else "Render needs attention",
                elapsed=round(time.monotonic() - started, 1),
            )
    finally:
        process_alive = False
        try:
            if process is not None:
                try:
                    try:
                        stop_process(process)
                    except (OSError, subprocess.TimeoutExpired):
                        logging.exception("Unable to stop the render process tree")
                        if process.poll() is None:
                            process.kill()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                except (OSError, subprocess.SubprocessError):
                    logging.exception("Unable to reap the render process")
                    if process.poll() is None:
                        try:
                            process.kill()
                            process.wait(timeout=5)
                        except (OSError, subprocess.SubprocessError):
                            logging.exception("Unable to kill the render process")
                if monitor is not None:
                    monitor.join(timeout=5)
                process_alive = process.poll() is None
                job_tree = process.__dict__.get("_studio_job")
                if job_tree is not None:
                    try:
                        process_alive = process_alive or job_tree.active()
                    except OSError:
                        process_alive = True
                        logging.exception(
                            "Unable to confirm render helpers have exited"
                        )
                try:
                    release_process_tree(process)
                except OSError:
                    logging.exception("Unable to close the render process job")
                if process.stdin is not None:
                    try:
                        process.stdin.close()
                    except OSError:
                        logging.exception("Unable to close render startup input")
                if process.stdout is not None:
                    try:
                        process.stdout.close()
                    except OSError:
                        logging.exception("Unable to close render output")
            # Per-render media is never reused. Release it after the worker exits.
            base = RENDERS.resolve()
            if (
                not process_alive
                and directory.resolve().parent == base
                and re.fullmatch(r"[0-9a-f]{32}", job_id)
            ):
                try:
                    shutil.rmtree(directory)
                except OSError:
                    logging.exception("Unable to remove temporary render media")
            for path in staged_files:
                try:
                    path.unlink(missing_ok=True)
                except OSError:
                    logging.exception("Unable to remove a staged render file")
        finally:
            with LOCK:
                job.pop("log", None)
                # Never permit another render while an unkillable process survives.
                if ACTIVE == job_id and not process_alive:
                    ACTIVE = None
                if PROCESS is process and not process_alive:
                    PROCESS = None
                if WORKER is threading.current_thread():
                    WORKER = None
                publish("render", {"render": render_summary(job)})
                if not ACTIVE:
                    start_next_render()
            if not process_alive:
                VIDEOS.render_finished(render_summary(job))
                settle_queue()


def queue_paused() -> bool:
    """Whether waiting videos are held instead of starting one after another."""
    if QUEUE_PAUSED is not None:
        return QUEUE_PAUSED
    return AGENTS.settings.get("queue_start", "auto") == "manual"


def waiting_jobs() -> list[dict]:
    """Queued renders in the order they will start. Call under LOCK."""
    return sorted(
        (
            job
            for job in JOBS.values()
            if job["status"] == "queued" and "_source" in job
        ),
        key=lambda job: job.get("_seq", 0),
    )


def queue_state() -> dict:
    """Call under LOCK."""
    settings = AGENTS.settings
    return {
        "paused": queue_paused(),
        "start": settings.get("queue_start", "auto"),
        "on_error": settings.get("queue_on_error", "fix"),
        "active": ACTIVE,
        "waiting": [job["id"] for job in waiting_jobs()],
    }


def publish_queue():
    """Call under LOCK."""
    publish("queue", {"queue": queue_state()})


def set_queue_paused(paused: bool):
    global QUEUE_PAUSED
    with LOCK:
        QUEUE_PAUSED = paused
        if not paused:
            start_next_render()
        publish_queue()


def reset_queue():
    global QUEUE_PAUSED
    QUEUE_PAUSED = None


def settle_queue():
    """Return a started queue to its preference once nothing is left to do.

    Call without LOCK held: it asks the video workflows whether any video is
    still being fixed, because a repaired video re-joins the running queue.
    """
    global QUEUE_PAUSED
    busy = VIDEOS.pending()
    with LOCK:
        if QUEUE_PAUSED is False and not busy and not ACTIVE and not waiting_jobs():
            QUEUE_PAUSED = None
            publish_queue()


def start_next_render():
    """Called under LOCK, after the previous process and media are released."""
    global ACTIVE, WORKER
    if ACTIVE or UPDATE_INSTALLING or queue_paused():
        return
    for job in waiting_jobs():
        source = job.pop("_source")
        worker = threading.Thread(
            target=render_worker, args=(job["id"], source), daemon=True
        )
        ACTIVE, WORKER = job["id"], worker
        job["stage"] = "Preparing your script…"
        try:
            worker.start()
        except RuntimeError:
            ACTIVE, WORKER = None, None
            job.update(
                status="failed",
                stage="Couldn't start render",
                error="The render worker could not start. Try again.",
            )
            publish("render", {"render": render_summary(job)})
            continue
        publish("render", {"render": render_summary(job)})
        break
    publish_queue()


def render_watermark(body: RenderRequest) -> dict | None:
    """Snapshot the watermark a render will use when it is requested."""
    if body.watermark is not None:
        settings = body.watermark.model_dump()
    elif body.project is not None:
        own, parent = project_call(
            lambda: projects.watermark_settings(
                project_root(), body.project, body.workspace
            )
        )
        settings, _ = watermark.effective(ROOT, own, parent)
    else:
        settings = watermark.load_default(ROOT)
    try:
        return watermark.render_spec(ROOT, settings)
    except watermark.WatermarkError as error:
        raise HTTPException(422, str(error)) from error


@app.post("/api/render", status_code=202)
def start_render(body: RenderRequest):
    global ACTIVE, WORKER
    if body.quality not in QUALITY:
        raise HTTPException(422, "Choose Preview, Standard, or High quality.")
    analysis = analyze(body.source)
    if analysis["error"]:
        raise HTTPException(422, analysis["error"])
    if body.scene not in analysis["scenes"]:
        raise HTTPException(422, "Choose a scene from this script.")
    if analysis["needs_latex"] and not health()["latex"]:
        raise HTTPException(
            422,
            "This script uses Tex or MathTex. Install the Studio edition with LaTeX, or install MiKTeX and restart Studio. You can also rewrite equations using MathTypst.",
        )
    if analysis["needs_typst"] and not health()["typst"]:
        raise HTTPException(
            422,
            "This script uses Typst or MathTypst. Reinstall Studio to repair Typst support.",
        )
    project = None
    if body.project is not None:
        try:
            project = projects.read(ROOT / "projects", body.project, body.workspace)
        except KeyError as error:
            raise HTTPException(
                404, "This project no longer exists. Choose another project."
            ) from error
    narration_job = None
    if body.narration is not None and body.narration.enabled:
        try:
            narration.language(body.narration.voice)
            narration.required_files(body.narration.quality)
            segments = narration.parse_timeline(body.narration.text)
        except narration.NarrationError as error:
            raise HTTPException(422, str(error)) from error
        if segments:
            quality = narration.usable_quality(ROOT, body.narration.quality)
            if not narration.installed(ROOT, quality):
                raise HTTPException(
                    422,
                    "Download the narration voice in the Narration tab, or turn narration off.",
                )
            narration_job = body.narration.model_dump(exclude={"enabled"})
            narration_job["quality"] = quality
    watermark_job = render_watermark(body)
    with LOCK:
        if UPDATE_INSTALLING:
            raise HTTPException(
                409, "Studio is preparing an update. Wait for it to restart."
            )
        if ACTIVE and not body.queue:
            raise HTTPException(
                409,
                "A video is already rendering. Wait for it to finish or cancel it first.",
            )
        if sum(job["status"] == "queued" for job in JOBS.values()) >= 20:
            raise HTTPException(
                409, "The render queue is full. Wait for a video to finish."
            )
        job_id = uuid.uuid4().hex
        name = body.name.strip() or body.scene
        folder = reserve_output_folder(name)
        job = {
            "id": job_id,
            "name": name,
            "folder": folder,
            "scene": body.scene,
            "quality": body.quality,
            "created": datetime.now(timezone.utc).isoformat(),
            "status": "queued",
            "stage": "Waiting for you to start the queue"
            if not ACTIVE and body.queue and queue_paused()
            else "Waiting in render queue"
            if ACTIVE
            else "Preparing your script…",
            "log": "",
            "elapsed": 0,
            "_source_hash": hashlib.sha256(analysis["source"].encode()).hexdigest(),
        }
        if body._workflow:
            job["workflow"] = body._workflow
        if project is not None:
            job.update(project=project["id"], project_name=project["name"])
            if body.workspace:
                job["workspace"] = body.workspace
        if narration_job is not None:
            job.update(narrated=True, _narration=narration_job)
        if watermark_job is not None:
            job.update(watermarked=True, _watermark=watermark_job)
        if body.width is not None or body.fps is not None:
            preset_width, preset_height = QUALITY_DIMENSIONS[body.quality]
            job.update(
                width=body.width if body.width is not None else preset_width,
                height=body.height if body.height is not None else preset_height,
                fps=body.fps if body.fps is not None else QUALITY[body.quality][2],
            )
            if body.fps is not None:
                job["_requested_fps"] = body.fps
                job["_requested_rate"] = str(frame_rate_fraction(body.fps))
            job["quality"] = output_quality(job["width"], job["height"], job["fps"])
        if ACTIVE or (body.queue and queue_paused()):
            job["_source"] = analysis["source"]
            job["_seq"] = next(QUEUE_SEQ)
            JOBS[job_id] = job
            publish("render", {"render": render_summary(job)})
            publish_queue()
            return public_job(job)
        try:
            worker = threading.Thread(
                target=render_worker, args=(job_id, analysis["source"]), daemon=True
            )
            JOBS[job_id] = job
            ACTIVE = job_id
            WORKER = worker
            worker.start()
        except RuntimeError as error:
            JOBS.pop(job_id, None)
            ACTIVE = None
            WORKER = None
            try:
                (OUTPUTS / folder).rmdir()
            except OSError:
                logging.exception("Unable to remove the unused render folder")
            raise HTTPException(
                503, "The render worker could not start. Try again."
            ) from error
        return public_job(job)


@app.get("/api/renders/{job_id}")
def get_render(job_id: str):
    with LOCK:
        if job_id not in JOBS:
            raise HTTPException(404, "Render not found.")
        job = JOBS[job_id]
        result = job_snapshot(job)
        if "log" not in result:
            try:
                with output_file(job, "render.log").open("rb") as log:
                    log.seek(max(0, log.seek(0, 2) - 160_000))
                    result["log"] = log.read().decode("utf-8", errors="replace")[
                        -40_000:
                    ]
            except (OSError, HTTPException):
                result["log"] = ""
        return result


@app.post("/api/renders/{job_id}/cancel")
def cancel_render(job_id: str):
    with LOCK:
        job = JOBS.get(job_id)
        if not job or job["status"] not in {"queued", "rendering"}:
            raise HTTPException(409, "This render is no longer running.")
        if ACTIVE != job_id:
            job.update(status="cancelled", stage="Removed from render queue")
            job.pop("_source", None)
            publish("render", {"render": render_summary(job)})
            process = None
        else:
            JOBS[job_id]["_cancel"] = True
            process = PROCESS
    if process is not None:
        stop_process(process)
    if job["status"] == "cancelled":
        VIDEOS.render_finished(render_summary(job))
        with LOCK:
            publish_queue()
        settle_queue()
    return {"ok": True}


class MoveRequest(BaseModel):
    to: str = Field(pattern="^(front|up|down|back)$")


@app.get("/api/queue")
def get_queue():
    with LOCK:
        return queue_state()


@app.post("/api/queue/start")
def start_queue():
    set_queue_paused(False)
    settle_queue()
    with LOCK:
        return queue_state()


@app.post("/api/queue/pause")
def pause_queue():
    set_queue_paused(True)
    with LOCK:
        return queue_state()


@app.post("/api/queue/clear")
def clear_queue():
    with LOCK:
        waiting = [job["id"] for job in waiting_jobs()]
    for job_id in waiting:
        try:
            cancel_render(job_id)
        except HTTPException:
            continue  # it started or finished in the meantime
    with LOCK:
        return queue_state()


@app.post("/api/renders/{job_id}/move")
def move_render(job_id: str, body: MoveRequest):
    with LOCK:
        order = [job["id"] for job in waiting_jobs()]
        if job_id not in order:
            raise HTTPException(409, "This video is no longer waiting in the queue.")
        index = order.index(job_id)
        target = {
            "front": 0,
            "up": max(0, index - 1),
            "down": min(len(order) - 1, index + 1),
            "back": len(order) - 1,
        }[body.to]
        order.insert(target, order.pop(index))
        for waiting_id in order:
            JOBS[waiting_id]["_seq"] = next(QUEUE_SEQ)
        publish_queue()
        return queue_state()


@app.get("/api/renders/{job_id}/{kind}")
def render_file(job_id: str, kind: str, download: bool = False):
    with LOCK:
        job = JOBS.get(job_id)
        if not job:
            raise HTTPException(404, "Render not found.")
    filenames = {"video": "video.mp4", "script": "script.py", "log": "render.log"}
    if kind not in filenames:
        raise HTTPException(404, "File not found.")
    if kind == "video" and job["status"] != "completed":
        raise HTTPException(404, "This file isn't ready yet.")
    path = output_file(job, filenames[kind])
    if not path.is_file():
        raise HTTPException(404, "This file isn't ready yet.")
    safe_name = re.sub(r"[^\w .-]", "", job["name"]).strip() or "animation"
    filename = safe_name + (
        ".mp4" if kind == "video" else ".py" if kind == "script" else "-log.txt"
    )
    return FileResponse(
        path,
        media_type="video/mp4" if kind == "video" else "text/plain",
        filename=filename if download else None,
    )


def reveal(path: Path):
    path.mkdir(parents=True, exist_ok=True)
    if os.name == "nt":
        os.startfile(str(path))
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(path)])
    else:
        subprocess.Popen(["xdg-open", str(path)])
    return {"ok": True, "path": str(path)}


@app.post("/api/open/{folder}")
def open_folder(folder: str):
    if folder not in {"outputs", "assets"}:
        raise HTTPException(404, "Folder not found.")
    return reveal(OUTPUTS if folder == "outputs" else ROOT / "assets")


class ProjectCreate(BaseModel):
    name: str = Field(default="Untitled project", max_length=100)
    source: str = Field(default="", max_length=500_000)
    style: str | None = Field(default=None, max_length=64)


class ProjectUpdate(BaseModel):
    name: str | None = Field(default=None, max_length=100)
    source: str | None = Field(default=None, max_length=500_000)
    narration: NarrationSettings | None = None
    # A house style id, or "" to follow the default (or parent project) style.
    style: str | None = Field(default=None, max_length=64)


def project_root() -> Path:
    return ROOT / "projects"


def project_call(action):
    try:
        return action()
    except KeyError as error:
        raise HTTPException(
            404, "This project no longer exists. Choose another project."
        ) from error
    except projects.ProjectError as error:
        raise HTTPException(409, str(error)) from error


@app.get("/api/projects")
def list_projects():
    return {"projects": projects.list_projects(project_root())}


@app.post("/api/projects")
def create_project(body: ProjectCreate):
    check_style(body.style)
    return project_call(
        lambda: projects.create(
            project_root(), body.name, body.source, style=body.style or None
        )
    )


@app.post("/api/projects/import")
async def import_project(request: Request):
    async def limited_stream():
        size = 0
        async for chunk in request.stream():
            size += len(chunk)
            if size > projects.MAX_PACKAGE_BYTES + 2 * 1024**2:
                raise MultiPartException("Keep project uploads under 512 MB.")
            yield chunk

    if not request.headers.get("content-type", "").startswith("multipart/form-data"):
        raise HTTPException(422, "Choose a project folder or ZIP to import.")
    try:
        form = await MultiPartParser(
            request.headers,
            limited_stream(),
            max_files=projects.MAX_PACKAGE_FILES,
            max_fields=0,
        ).parse()
    except MultiPartException as error:
        raise HTTPException(422, str(error)) from error
    try:
        files = [
            value
            for key, value in form.multi_items()
            if key == "files" and isinstance(value, UploadFile)
        ]
        if not files:
            raise HTTPException(422, "Choose a project folder or ZIP to import.")
        if len(files) == 1 and (files[0].filename or "").lower().endswith(".zip"):
            action = lambda: projects.import_zip(
                project_root(), files[0].file, Path(files[0].filename).stem
            )
        else:
            action = lambda: projects.import_package(
                project_root(),
                [(file.filename or "", file.file, file.size or 0) for file in files],
            )
        return await run_in_threadpool(project_call, action)
    except OSError as error:
        raise HTTPException(
            507,
            "Couldn’t save the imported project. Check free disk space and folder permissions, then retry.",
        ) from error
    finally:
        await form.close()


@app.post("/api/projects/from-pdf", status_code=201)
async def project_from_pdf(request: Request, project: str | None = None):
    """One script PDF per isolated video project, with a readable reference copy."""
    from pypdf import PdfReader
    from pypdf.errors import PyPdfError

    filename = unquote(request.headers.get("X-Filename", "")).strip()
    if not filename.lower().endswith(".pdf"):
        raise HTTPException(422, "Choose a PDF script for this video.")
    data = bytearray()
    async for chunk in request.stream():
        data.extend(chunk)
        if len(data) > 20 * 1024**2:
            raise HTTPException(413, "Each script PDF must be smaller than 20 MB.")

    def create():
        try:
            reader = PdfReader(io.BytesIO(data))
            if reader.is_encrypted:
                raise HTTPException(422, "Unlock this PDF before adding it.")
            if len(reader.pages) > 300:
                raise HTTPException(422, "Script PDFs can have up to 300 pages.")
            sections, length = [], 0
            for index, page in enumerate(reader.pages):
                text = page.extract_text() or ""
                length += len(text)
                if length > 500_000:
                    raise HTTPException(
                        422,
                        "This PDF contains too much text. Split it into individual video scripts.",
                    )
                sections.append(f"Page {index + 1}\n{text}")
            if not length or not any(
                section.split("\n", 1)[1].strip() for section in sections
            ):
                raise HTTPException(
                    422,
                    "This PDF has no readable text. Use a PDF with selectable text.",
                )
        except (PyPdfError, ValueError, OSError) as error:
            raise HTTPException(
                422, "Couldn't read this PDF. Export it again and retry."
            ) from error
        result = (
            projects.create_video(project_root(), project, Path(filename).stem)
            if project
            else projects.create(
                project_root(),
                Path(filename).stem,
                narration={"enabled": True, "quality": "high"},
            )
        )
        directory = projects.assets_dir(
            project_root(), result["id"], result.get("workspace")
        )
        try:
            (directory / "video-script.pdf").write_bytes(data)
            (directory / "video-script.txt").write_text(
                "\n\n".join(sections), encoding="utf-8"
            )
        except OSError:
            if result.get("workspace"):
                path = projects.workspace_folder(
                    project_root(), result["id"], result["workspace"]
                )
                projects.delete(path.parent, result["workspace"])
            else:
                projects.delete(project_root(), result["id"])
            raise
        return result

    return await run_in_threadpool(project_call, create)


@app.get("/api/projects/{project_id}")
def read_project(project_id: str, workspace: str | None = None):
    return project_call(lambda: projects.read(project_root(), project_id, workspace))


@app.post("/api/projects/{project_id}")
def update_project(project_id: str, body: ProjectUpdate, workspace: str | None = None):
    if (
        body.source is not None or body.narration is not None
    ) and AGENTS.busy_workspace(project_id, workspace):
        raise HTTPException(
            409, "Wait for the agent to finish before editing this project's files."
        )
    result = project_call(
        lambda: projects.update(
            project_root(),
            project_id,
            name=body.name,
            source=body.source,
            narration=body.narration.model_dump() if body.narration else None,
            workspace=workspace,
            style=check_style(body.style),
        )
    )
    if body.style is not None:
        refresh_style_module(project_id, workspace)
    return result


@app.delete("/api/projects/{project_id}")
def delete_project(project_id: str):
    with LOCK:
        if any(
            job.get("project") == project_id
            and (job_id == ACTIVE or job.get("status") in {"queued", "rendering"})
            for job_id, job in JOBS.items()
        ):
            raise HTTPException(
                409, "Finish or cancel this project's queued renders first."
            )
    if AGENTS.busy_project(project_id):
        raise HTTPException(409, "Stop the agent working on this project first.")
    replacement = project_call(lambda: projects.delete(project_root(), project_id))
    AGENTS.delete_project(project_id)
    return {"ok": True, "replacement": replacement}


@app.post("/api/projects/{project_id}/open-assets")
def open_project_assets(project_id: str, workspace: str | None = None):
    return reveal(
        project_call(lambda: projects.assets_dir(project_root(), project_id, workspace))
    )


@app.post("/api/projects/{project_id}/open-folder")
def open_project_folder(project_id: str):
    return reveal(project_call(lambda: projects.folder(project_root(), project_id)))


@app.get("/api/projects/{project_id}/assets")
def list_project_assets(project_id: str, workspace: str | None = None):
    directory = project_call(
        lambda: projects.assets_dir(project_root(), project_id, workspace)
    )
    return {"assets": projects.list_assets(directory)}


@app.post("/api/projects/{project_id}/assets")
async def upload_project_asset(
    project_id: str, request: Request, workspace: str | None = None
):
    filename = unquote(request.headers.get("X-Filename", "")).strip()
    if not filename:
        raise HTTPException(422, "Choose a file to add.")
    directory = project_call(
        lambda: projects.assets_dir(project_root(), project_id, workspace)
    )
    with projects.LOCK:
        target = projects.reserve_asset(directory, filename)
        partial = target.with_name(target.name + ".part")
        output = partial.open("xb")
    size = 0
    try:
        with output:
            async for chunk in request.stream():
                size += len(chunk)
                if size > MAX_ASSET_BYTES:
                    raise HTTPException(413, "Assets must be smaller than 2 GB.")
                output.write(chunk)
        partial.replace(target)
    finally:
        partial.unlink(missing_ok=True)
    return {
        "path": target.relative_to(directory).as_posix(),
        "size": size,
        "kind": projects.asset_kind(target),
    }


class AssetPickerRequest(BaseModel):
    kind: str = "documents"

    @field_validator("kind")
    @classmethod
    def valid_kind(cls, value):
        if value not in {"documents", "media"}:
            raise ValueError("Choose documents or media.")
        return value


@app.post("/api/projects/{project_id}/assets/choose")
def choose_project_assets(
    project_id: str, body: AssetPickerRequest, workspace: str | None = None
):
    directory = project_call(
        lambda: projects.assets_dir(project_root(), project_id, workspace)
    )
    try:
        selected = file_picker.choose(body.kind)
    except file_picker.PickerError as error:
        raise HTTPException(409, str(error)) from error
    assets, errors = [], []
    for source in selected:
        partial = None
        try:
            with source.open("rb") as original:
                if not source.is_file():
                    raise ValueError("Choose a file, not a folder.")
                with projects.LOCK:
                    target = projects.reserve_asset(directory, source.name)
                    partial = target.with_name(target.name + ".part")
                    output = partial.open("xb")
                size = 0
                with output:
                    while chunk := original.read(1024 * 1024):
                        size += len(chunk)
                        if size > MAX_ASSET_BYTES:
                            raise ValueError("Assets must be smaller than 2 GB.")
                        output.write(chunk)
                partial.replace(target)
                assets.append(
                    {
                        "path": target.relative_to(directory).as_posix(),
                        "size": size,
                        "kind": projects.asset_kind(target),
                    }
                )
        except (OSError, ValueError) as error:
            errors.append({"file": source.name, "message": str(error)})
        finally:
            if partial:
                partial.unlink(missing_ok=True)
    return {"assets": assets, "errors": errors}


def project_asset(project_id: str, path: str, workspace: str | None = None) -> Path:
    directory = project_call(
        lambda: projects.assets_dir(project_root(), project_id, workspace)
    )
    try:
        file = projects.asset_file(directory, path)
    except KeyError as error:
        raise HTTPException(404, "Asset not found.") from error
    if not file.is_file():
        raise HTTPException(404, "Asset not found.")
    return file


@app.get("/api/projects/{project_id}/assets/{path:path}")
def read_project_asset(project_id: str, path: str, workspace: str | None = None):
    return FileResponse(project_asset(project_id, path, workspace))


@app.delete("/api/projects/{project_id}/assets/{path:path}")
def delete_project_asset(project_id: str, path: str, workspace: str | None = None):
    project_asset(project_id, path, workspace).unlink()
    return {"ok": True}


# ------------------------------------------------------------------ watermark


class WatermarkUpdate(BaseModel):
    # None removes the override, so the project (or the default) applies again.
    settings: WatermarkSettings | None = None


class WatermarkAsset(BaseModel):
    path: str = Field(min_length=1, max_length=1000)


def require_logo(settings: WatermarkSettings | None):
    if settings is not None and settings.type == "image":
        if not watermark.logo_path(ROOT, settings.logo).is_file():
            raise HTTPException(
                422, "The watermark logo is missing. Upload or choose it again."
            )


def store_watermark_logo(data: bytes, filename: str) -> dict:
    try:
        return watermark.store_logo(ROOT, data, filename)
    except watermark.WatermarkError as error:
        raise HTTPException(422, str(error)) from error
    except OSError as error:
        raise HTTPException(
            507, "Couldn’t save the logo. Check free disk space, then retry."
        ) from error


@app.get("/api/watermark")
def watermark_default():
    return {"settings": watermark.load_default(ROOT)}


@app.post("/api/watermark")
def save_watermark_default(body: WatermarkSettings):
    require_logo(body)
    try:
        return {"settings": watermark.save_default(ROOT, body.model_dump())}
    except OSError as error:
        raise HTTPException(
            507, "Couldn’t save the watermark. Check folder permissions, then retry."
        ) from error


@app.post("/api/watermark/logos")
async def upload_watermark_logo(request: Request):
    filename = unquote(request.headers.get("X-Filename", "")).strip()
    if Path(filename).suffix.lower() not in watermark.LOGO_SUFFIXES:
        raise HTTPException(422, "Choose a PNG, JPG, WebP, GIF, BMP or SVG logo.")
    data = bytearray()
    async for chunk in request.stream():
        data.extend(chunk)
        if len(data) > watermark.MAX_LOGO_BYTES:
            raise HTTPException(413, "Logos must be smaller than 10 MB.")
    return await run_in_threadpool(store_watermark_logo, bytes(data), filename)


@app.get("/api/watermark/logos/{logo}")
def read_watermark_logo(logo: str):
    try:
        path = watermark.logo_path(ROOT, logo)
    except KeyError as error:
        raise HTTPException(404, "Logo not found.") from error
    if not path.is_file():
        raise HTTPException(404, "Logo not found.")
    return FileResponse(path, media_type="image/png")


@app.get("/api/projects/{project_id}/watermark")
def project_watermark(project_id: str, workspace: str | None = None):
    own, parent = project_call(
        lambda: projects.watermark_settings(project_root(), project_id, workspace)
    )
    settings, source = watermark.effective(ROOT, own, parent)
    inherited, inherited_from = watermark.effective(ROOT, None, parent)
    return {
        "own": watermark.clean(own),
        "settings": settings,
        "source": source,
        "inherited": inherited,
        "inherited_from": inherited_from,
    }


@app.post("/api/projects/{project_id}/watermark")
def save_project_watermark(
    project_id: str, body: WatermarkUpdate, workspace: str | None = None
):
    require_logo(body.settings)
    project_call(
        lambda: projects.set_watermark(
            project_root(),
            project_id,
            workspace,
            body.settings.model_dump() if body.settings is not None else None,
        )
    )
    return project_watermark(project_id, workspace)


@app.post("/api/projects/{project_id}/watermark/logo")
def watermark_logo_from_asset(
    project_id: str, body: WatermarkAsset, workspace: str | None = None
):
    file = project_asset(project_id, body.path, workspace)
    if file.suffix.lower() not in watermark.LOGO_SUFFIXES:
        raise HTTPException(422, "Choose a PNG, JPG, WebP, GIF, BMP or SVG logo.")
    try:
        if file.stat().st_size > watermark.MAX_LOGO_BYTES:
            raise HTTPException(413, "Logos must be smaller than 10 MB.")
        data = file.read_bytes()
    except OSError as error:
        raise HTTPException(404, "Asset not found.") from error
    return store_watermark_logo(data, file.name)


# ------------------------------------------------------------------ house styles


class StyleCreate(BaseModel):
    style: dict = Field(default_factory=dict)
    # Duplicate this style (built-in or custom) and apply the changes on top.
    source: str | None = Field(default=None, max_length=64)


class StyleUpdate(BaseModel):
    style: dict


class StyleDefault(BaseModel):
    id: str = Field(max_length=64)


def style_call(action):
    # styles.py serializes its own library writes; the render LOCK is not needed.
    try:
        return action()
    except KeyError as error:
        raise HTTPException(404, "This style no longer exists.") from error
    except styles.StyleError as error:
        raise HTTPException(422, str(error)) from error
    except OSError as error:
        raise HTTPException(500, "Couldn't save the style library.") from error


def check_style(style_id: str | None) -> str | None:
    if style_id and not any(
        style["id"] == style_id for style in styles.library(ROOT)["styles"]
    ):
        raise HTTPException(422, "Choose a style from the Styles list.")
    return style_id


def refresh_style_module(project_id: str, workspace: str | None = None):
    try:
        styles.write_module(
            ROOT, projects.workspace_folder(project_root(), project_id, workspace)
        )
    except (KeyError, OSError):
        logging.exception("Unable to write the project style module")


@app.get("/api/styles")
def list_styles():
    return styles.library(ROOT)


@app.post("/api/styles", status_code=201)
def create_style(body: StyleCreate):
    style = style_call(lambda: styles.create(ROOT, body.style, body.source))
    return style


@app.post("/api/styles/default")
def default_style(body: StyleDefault):
    result = style_call(lambda: styles.set_default(ROOT, body.id))
    return result


@app.post("/api/styles/{style_id}")
def update_style(style_id: str, body: StyleUpdate):
    style = style_call(lambda: styles.update(ROOT, style_id, body.style))
    return style


@app.delete("/api/styles/{style_id}")
def delete_style(style_id: str):
    result = style_call(lambda: styles.delete(ROOT, style_id))
    return result


@app.get("/api/fonts")
def list_fonts(project: str | None = None, workspace: str | None = None):
    """Installed font families, plus font files uploaded to a project's assets."""
    uploaded = []
    if project is not None:
        folder = project_call(
            lambda: projects.workspace_folder(project_root(), project, workspace)
        )
        uploaded = styles.project_fonts(folder)
    return {"system": styles.system_fonts(), "project": uploaded}


class NarrationDownload(BaseModel):
    quality: str = "high"


def narration_status():
    return {
        "voices": narration.voices(),
        "models": {
            quality: {
                "installed": narration.installed(ROOT, quality),
                "download_size": narration.download_size(ROOT, quality),
            }
            for quality in ("standard", "high")
        },
        "download": NARRATION.snapshot(),
    }


@app.get("/api/narration")
def get_narration():
    return narration_status()


@app.post("/api/narration/download")
def download_narration(body: NarrationDownload):
    try:
        NARRATION.start(body.quality)
    except narration.NarrationError as error:
        raise HTTPException(409, str(error)) from error
    return narration_status()


@app.post("/api/narration/cancel")
def cancel_narration_download():
    NARRATION.cancel()
    return narration_status()


@app.post("/api/narration/preview")
def preview_narration(body: NarrationSettings):
    try:
        narration.language(body.voice)
        narration.required_files(body.quality)
    except narration.NarrationError as error:
        raise HTTPException(422, str(error)) from error
    body.quality = narration.usable_quality(ROOT, body.quality)
    if not narration.installed(ROOT, body.quality):
        raise HTTPException(422, "Download the narration voice first.")
    folder = RENDERS / f"voice-{uuid.uuid4().hex}"
    folder.mkdir(parents=True, exist_ok=True)
    try:
        spec = folder / "spec.json"
        spec.write_text(
            json.dumps({**body.model_dump(), "data_root": str(ROOT)}), encoding="utf-8"
        )
        output = folder / "preview.wav"
        try:
            result = subprocess.run(
                [
                    renderer_python(),
                    "-u",
                    str(APP_ROOT / "narration.py"),
                    "preview",
                    "--spec",
                    str(spec),
                    "--output",
                    str(output),
                ],
                cwd=APP_ROOT,
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=180,
                creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
            )
        except subprocess.TimeoutExpired as error:
            raise HTTPException(504, "The voice preview took too long.") from error
        if result.returncode or not output.is_file():
            detail = (result.stderr or result.stdout).strip().splitlines()
            raise HTTPException(
                422,
                detail[-1][:300] if detail else "The voice preview could not be made.",
            )
        return Response(output.read_bytes(), media_type="audio/wav")
    finally:
        shutil.rmtree(folder, ignore_errors=True)


class AgentSettings(BaseModel):
    providers: dict[str, dict] | None = None
    default_provider: str | None = Field(default=None, max_length=20)
    access: str | None = Field(default=None, max_length=20)
    auto_render: bool | None = None
    queue_start: str | None = Field(default=None, pattern="^(auto|manual)$")
    queue_on_error: str | None = Field(default=None, pattern="^(fix|skip|pause)$")
    auto_update_agents: bool | None = None


class ThreadCreate(BaseModel):
    workspace: str | None = Field(default=None, max_length=32)
    provider: str | None = Field(default=None, max_length=20)
    model: str | None = Field(default=None, max_length=100)
    effort: str | None = Field(default=None, max_length=20)
    title: str = Field(default="", max_length=200)


class ThreadUpdate(BaseModel):
    title: str | None = Field(default=None, max_length=200)
    provider: str | None = Field(default=None, max_length=20)
    model: str | None = Field(default=None, max_length=100)
    effort: str | None = Field(default=None, max_length=20)


class TurnRequest(BaseModel):
    text: str = Field(min_length=1, max_length=agents.MAX_PROMPT)
    context: str = Field(default="", max_length=agents.MAX_PROMPT)
    model: str | None = Field(default=None, max_length=100)
    effort: str | None = Field(default=None, max_length=20)
    mode: str = Field(default="build", pattern="^(build|plan)$")


class VideoRequest(RenderRequest):
    source: str = ""
    scene: str = ""
    quality: str = "standard"
    thread: str | None = Field(default=None, max_length=32)
    repair: bool = False
    render: str | None = Field(default=None, max_length=32)


def prepare_video(project: dict, options: dict, automatic: bool):
    analysis = analyze(project["source"])
    if analysis["error"]:
        raise VideoProblem(analysis["error"])
    runtime = health()
    if analysis["needs_latex"] and not runtime["latex"]:
        raise VideoProblem(
            "LaTeX and dvisvgm are unavailable. Replace Tex/MathTex with MathTypst or Text to keep the video renderable in this runtime."
        )
    if analysis["needs_typst"] and not runtime["typst"]:
        raise VideoProblem(
            "Typst is unavailable. Replace Typst/MathTypst with supported Text or available LaTeX primitives."
        )
    if not options:
        prefs = preferences()
        options = {"quality": prefs.get("manim-quality", "standard")}
        dimensions = re.fullmatch(r"(\d+)x(\d+)", prefs.get("manim-resolution", ""))
        if dimensions:
            options.update(width=int(dimensions[1]), height=int(dimensions[2]))
        if prefs.get("manim-fps"):
            options["fps"] = float(prefs["manim-fps"])
    settings = project["narration"] if options.get("narration", True) else None
    warning = None
    if settings and settings["enabled"] and settings["text"].strip():
        try:
            narration.parse_timeline(settings["text"])
        except narration.NarrationError as error:
            raise VideoProblem(str(error)) from error
        settings = {
            **settings,
            "quality": narration.usable_quality(ROOT, settings["quality"]),
        }
        if not narration.installed(ROOT, settings["quality"]):
            if not automatic:
                raise VideoProblem(
                    "Download the narration voice in the Narration tab, or turn narration off.",
                    repairable=False,
                )
            settings = None
            warning = "Rendered without narration because the voice model isn't downloaded. Download it in the Narration tab to add speech."
    scene = options.get("scene")
    return {
        **{
            key: options[key]
            for key in ("quality", "width", "height", "fps", "watermark")
            if options.get(key) is not None
        },
        "source": analysis["source"],
        "scene": scene if scene in analysis["scenes"] else analysis["scenes"][-1],
        "project": project["id"],
        "workspace": project.get("workspace"),
        "name": project["name"],
        "narration": settings,
        "queue": True,
        "warning": warning,
    }


def submit_video(values):
    values = dict(values)
    workflow = values.pop("workflow")
    body = RenderRequest(**values)
    body._workflow = workflow
    return start_render(body)


VIDEOS = VideoWorkflows(
    manager=lambda: AGENTS,
    read=lambda project, workspace: projects.read(project_root(), project, workspace),
    prepare=prepare_video,
    submit=submit_video,
    render=lambda render_id: get_render(render_id),
    cancel_render=lambda render_id: cancel_render(render_id),
    publish=publish,
    pause_queue=lambda: set_queue_paused(True),
)
AGENTS.on_turn_finished = VIDEOS.agent_finished


@app.get("/api/videos")
def video_status():
    return VIDEOS.snapshot()


@app.post("/api/projects/{project_id}/video", status_code=202)
def start_video(project_id: str, body: VideoRequest):
    if UPDATE_INSTALLING:
        raise HTTPException(
            409, "Studio is preparing an update. Wait for it to restart."
        )
    try:
        diagnostics = None
        if body.repair and body.render:
            previous = get_render(body.render)
            if (
                previous.get("project") != project_id
                or previous.get("workspace") != body.workspace
            ):
                raise HTTPException(422, "Choose the error from this video.")
            current = projects.read(project_root(), project_id, body.workspace)
            analysis = analyze(current["source"])
            with LOCK:
                original_hash = JOBS[body.render].get("_source_hash")
            if previous["status"] == "failed" and (
                not original_hash
                or hashlib.sha256(analysis["source"].encode()).hexdigest()
                == original_hash
            ):
                diagnostics = f"Render error: {previous.get('error', '')}\nProblem: {previous.get('problem') or ''}\nRender log:\n{previous.get('log', '')[-10000:]}"
        return VIDEOS.start(
            project_id,
            workspace=body.workspace,
            thread=body.thread,
            diagnostics=diagnostics,
            options={
                **body.model_dump(
                    include={"scene", "quality", "width", "height", "fps", "watermark"},
                    exclude_none=True,
                ),
                "narration": body.narration is None or body.narration.enabled,
            },
        )
    except KeyError as error:
        raise HTTPException(404, "This project or chat no longer exists.") from error
    except VideoProblem as error:
        raise HTTPException(409, str(error)) from error


@app.post("/api/videos/{video_id}/cancel")
def cancel_video(video_id: str):
    try:
        VIDEOS.cancel(video_id)
    except KeyError as error:
        raise HTTPException(404, "Video not found.") from error
    return {"ok": True}


def agent_call(action):
    try:
        return action()
    except KeyError as error:
        raise HTTPException(404, "This chat or project no longer exists.") from error
    except agents.AgentError as error:
        raise HTTPException(409, str(error)) from error


@app.get("/api/agents")
def agent_status():
    with AGENTS.lock:
        unchecked = not AGENTS.statuses and AGENTS.probe_thread is None
    return AGENTS.refresh() if unchecked else AGENTS.snapshot()


@app.post("/api/agents/refresh")
def refresh_agents():
    return AGENTS.refresh(wait=True)


@app.get("/api/usage")
def plan_usage(refresh: bool = False):
    """Codex and Claude plan limits, as the signed-in CLIs report them."""
    with AGENTS.lock:
        settings = json.loads(json.dumps(AGENTS.settings))
    return usage.snapshot(settings, force=refresh)


@app.post("/api/agents/settings")
def update_agent_settings(body: AgentSettings):
    result = AGENTS.update_settings(body.model_dump(exclude_none=True))
    with LOCK:
        # Switching to automatic start releases anything that was being held.
        start_next_render()
        publish_queue()
    if body.auto_update_agents is not None:
        AGENT_UPDATES.wake()
        AGENT_UPDATES.emit()
    return result


def agent_update_call(action):
    try:
        return action()
    except agent_updates.AgentUpdateError as error:
        raise HTTPException(409, str(error)) from error


@app.get("/api/agents/updates")
def agent_update_status():
    return AGENT_UPDATES.snapshot()


@app.post("/api/agents/updates/check")
def check_agent_updates():
    return agent_update_call(AGENT_UPDATES.check_now)


@app.post("/api/agents/{provider}/update")
def update_agent_cli(provider: str):
    return agent_update_call(lambda: AGENT_UPDATES.update_now(provider))


@app.post("/api/agents/{provider}/login")
def agent_login(provider: str):
    return agent_call(lambda: AGENTS.open_login(provider))


@app.post("/api/agents/{provider}/install")
def agent_install(provider: str):
    return agent_call(lambda: AGENTS.open_install(provider))


@app.get("/api/projects/{project_id}/threads")
def list_threads(project_id: str):
    return {"threads": agent_call(lambda: AGENTS.list_threads(project_id))}


@app.post("/api/projects/{project_id}/threads")
def create_thread(project_id: str, body: ThreadCreate):
    project_call(lambda: projects.folder(project_root(), project_id))
    return agent_call(lambda: AGENTS.create(project_id, **body.model_dump()))


@app.get("/api/projects/{project_id}/threads/{thread_id}")
def read_thread(project_id: str, thread_id: str):
    return agent_call(lambda: AGENTS.view(project_id, thread_id))


@app.post("/api/projects/{project_id}/threads/{thread_id}")
def update_thread(project_id: str, thread_id: str, body: ThreadUpdate):
    return agent_call(lambda: AGENTS.update(project_id, thread_id, **body.model_dump()))


class ThreadFork(BaseModel):
    turn: str | None = Field(default=None, max_length=40)
    title: str = Field(default="", max_length=80)
    provider: str | None = Field(default=None, max_length=20)
    model: str | None = Field(default=None, max_length=100)
    effort: str | None = Field(default=None, max_length=20)


@app.post("/api/projects/{project_id}/threads/{thread_id}/fork")
def fork_thread(project_id: str, thread_id: str, body: ThreadFork):
    return agent_call(
        lambda: AGENTS.fork(
            project_id,
            thread_id,
            turn_id=body.turn,
            title=body.title,
            provider=body.provider,
            model=body.model,
            effort=body.effort,
        )
    )


@app.delete("/api/projects/{project_id}/threads/{thread_id}")
def delete_thread(project_id: str, thread_id: str):
    agent_call(lambda: AGENTS.delete(project_id, thread_id))
    return {"ok": True}


@app.post("/api/projects/{project_id}/threads/{thread_id}/turns", status_code=202)
def start_turn(project_id: str, thread_id: str, body: TurnRequest):
    thread = agent_call(lambda: AGENTS.get(project_id, thread_id))
    if VIDEOS.busy(project_id, thread.get("workspace")):
        raise HTTPException(
            409, "Wait for this video to finish or stop it before sending changes."
        )
    with LOCK:
        if any(
            active.get("project") == project_id
            and (job_id == ACTIVE or active.get("status") in {"queued", "rendering"})
            and (
                thread.get("workspace") is None
                or active.get("workspace") in {None, thread.get("workspace")}
            )
            for job_id, active in JOBS.items()
        ):
            raise HTTPException(409, "Wait for this project's render to finish.")
    return agent_call(
        lambda: AGENTS.send(
            project_id,
            thread_id,
            body.text,
            model=body.model,
            effort=body.effort,
            context=body.context,
            mode=body.mode,
        )
    )


@app.post("/api/projects/{project_id}/threads/{thread_id}/interrupt")
def interrupt_turn(project_id: str, thread_id: str):
    return agent_call(lambda: AGENTS.interrupt(project_id, thread_id))


class StoryboardUpdate(BaseModel):
    text: str = Field(max_length=storyboard.MAX_TEXT)


class StoryboardApproval(BaseModel):
    thread: str | None = Field(default=None, max_length=32)


def storyboard_folder(project_id: str, workspace: str | None) -> Path:
    return project_call(
        lambda: projects.workspace_folder(project_root(), project_id, workspace)
    )


def storyboard_call(project_id: str, workspace: str | None, action):
    if AGENTS.busy_workspace(project_id, workspace):
        raise HTTPException(
            409, "Wait for the agent to finish before changing the storyboard."
        )
    try:
        result = action(storyboard_folder(project_id, workspace))
    except storyboard.StoryboardError as error:
        raise HTTPException(422, str(error)) from error
    publish(
        "project.changed",
        {"project": project_id, "files": [storyboard.FILE], "workspace": workspace},
    )
    return result


@app.get("/api/projects/{project_id}/storyboard")
def read_storyboard(project_id: str, workspace: str | None = None):
    return storyboard.read(storyboard_folder(project_id, workspace))


@app.post("/api/projects/{project_id}/storyboard")
def save_storyboard(
    project_id: str, body: StoryboardUpdate, workspace: str | None = None
):
    return storyboard_call(
        project_id, workspace, lambda folder: storyboard.save(folder, body.text)
    )


@app.post("/api/projects/{project_id}/storyboard/approve")
def approve_storyboard(
    project_id: str, body: StoryboardApproval, workspace: str | None = None
):
    """Approve the storyboard and, with a chat, start building the video from it."""
    if body.thread:
        thread = agent_call(lambda: AGENTS.get(project_id, body.thread))
        if thread.get("workspace") != workspace:
            raise HTTPException(422, "Choose the chat for this video.")
    result = storyboard_call(project_id, workspace, storyboard.approve)
    turn = (
        start_turn(project_id, body.thread, TurnRequest(text=storyboard.BUILD_PROMPT))
        if body.thread
        else None
    )
    return {"storyboard": result, "turn": turn}


app.mount("/static", StaticFiles(directory=WEB, check_dir=False), name="static")
