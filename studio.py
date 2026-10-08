"""Local-only Manim Studio server. Scripts run in a separate Python process."""

from __future__ import annotations

import ast
import asyncio
import ctypes
from ctypes import wintypes
from contextlib import asynccontextmanager
from datetime import datetime, timezone
from fractions import Fraction
import importlib.metadata
import json
import logging
import math
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import subprocess
import sys
import threading
import time
import uuid
import io
import tokenize

from fastapi import FastAPI, HTTPException, Request
from fastapi.exception_handlers import request_validation_exception_handler
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from fastapi.middleware.trustedhost import TrustedHostMiddleware
from pydantic import BaseModel, Field, field_validator, model_validator
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

ROOT = DATA_ROOT
WEB = APP_ROOT / "studio_ui"
RENDERS = ROOT / ".renders" / "studio"
OUTPUTS = ROOT / "outputs" / "studio"
TOKEN = secrets.token_urlsafe(32)
QUALITY = {
    "preview": ("-ql", "480p", 15),
    "standard": ("-qm", "720p", 30),
    "final": ("-qh", "1080p", 60),
}
QUALITY_DIMENSIONS = {
    "preview": (854, 480),
    "standard": (1280, 720),
    "final": (1920, 1080),
}
MAX_CAIRO_DIMENSION = 32766
LOCK = threading.RLock()
JOBS: dict[str, dict] = {}
ACTIVE: str | None = None
PROCESS: subprocess.Popen | None = None
WORKER: threading.Thread | None = None
ANSI = re.compile(r"\x1b\[[0-9;]*[a-zA-Z]")
UPDATES = UpdateManager(
    APP_VERSION,
    "portable" if PORTABLE else "installed" if STANDALONE else "source",
    PACKAGE_ROOT,
    DATA_ROOT,
)
UPDATE_INSTALLING = False
UPDATE_HANDLER = None


def frame_rate_fraction(value: float) -> Fraction:
    rate = Fraction(str(value))
    if rate.numerator > 2**31 - 1 or rate.denominator > 2**31 - 1:
        rate = rate.limit_denominator(1_000_000)
    return rate


class Script(BaseModel):
    source: str = Field(min_length=1, max_length=500_000)


class RenderRequest(Script):
    scene: str = Field(min_length=1, max_length=200)
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
    "manim-draft",
    "manim-name",
    "manim-quality",
    "manim-resolution",
    "manim-fps",
    "manim-tour-seen",
    "manim-theme",
    "manim-check-updates",
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
    """Discover scenes without importing or executing pasted code."""
    source = source.strip()
    if source.startswith("```"):
        lines = source.splitlines()
        if lines[-1].strip() == "```":
            source = "\n".join(lines[1:-1])
    try:
        tree = ast.parse(source)
    except UnicodeEncodeError as error:
        raise HTTPException(
            422, "The script contains invalid Unicode text. Paste or import it again."
        ) from error
    except RecursionError:
        return {
            "source": source,
            "scenes": [],
            "error": "This script contains an expression that is too deeply nested. Simplify it before rendering.",
        }
    except SyntaxError as error:
        return {
            "source": source,
            "scenes": [],
            "error": f"Line {error.lineno}: {error.msg}",
            "line": error.lineno,
        }
    bases = {
        "Scene",
        "MovingCameraScene",
        "ThreeDScene",
        "ZoomedScene",
        "VectorScene",
        "LinearTransformationScene",
    }
    modules = {"manim"}
    for node in tree.body:
        if isinstance(node, ast.ImportFrom) and node.module == "manim":
            bases.update(
                alias.asname or alias.name
                for alias in node.names
                if alias.name in bases
            )
        elif isinstance(node, ast.Import):
            modules.update(
                alias.asname or alias.name
                for alias in node.names
                if alias.name == "manim"
            )
    scenes = []
    pending = [node for node in tree.body if isinstance(node, ast.ClassDef)]
    while pending:
        found = []
        for node in pending:
            if any(
                (isinstance(base, ast.Name) and base.id in bases)
                or (
                    isinstance(base, ast.Attribute)
                    and isinstance(base.value, ast.Name)
                    and base.value.id in modules
                    and base.attr in bases
                )
                for base in node.bases
            ):
                scenes.append(node.name)
                bases.add(node.name)
                found.append(node)
        if not found:
            break
        pending = [node for node in pending if node not in found]
    return {
        "source": source,
        "scenes": scenes,
        "error": None
        if scenes
        else "No Manim scene found. Include a class such as MainScene(Scene).",
        "needs_latex": any(
            isinstance(node, ast.Name)
            and node.id in {"Tex", "MathTex"}
            or isinstance(node, ast.Attribute)
            and node.attr in {"Tex", "MathTex"}
            for node in ast.walk(tree)
        ),
    }


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


def format_source(source: str) -> str:
    """Normalize code whitespace while preserving every string literal byte."""
    if source.startswith("```") and source.rstrip().endswith("```"):
        source = "\n".join(source.splitlines()[1:-1])
    # Parse first: an unfinished literal must never be transformed speculatively.
    original_tree = ast.parse(source)
    protected = set()
    indentation = {}
    depth = 0
    new_statement = True
    for token in tokenize.generate_tokens(io.StringIO(source).readline):
        if token.type == tokenize.INDENT:
            depth += 1
        elif token.type == tokenize.DEDENT:
            depth -= 1
        elif token.type == tokenize.NEWLINE:
            new_statement = True
        elif (
            token.type
            not in {
                tokenize.NL,
                tokenize.COMMENT,
                tokenize.ENCODING,
                tokenize.ENDMARKER,
            }
            and new_statement
        ):
            indentation[token.start[0]] = depth * 4
            new_statement = False
        if token.type == tokenize.STRING or tokenize.tok_name[token.type].startswith(
            "FSTRING"
        ):
            protected.update(range(token.start[0], token.end[0] + 1))
    lines = source.splitlines(keepends=True)
    for index, line in enumerate(lines, 1):
        if index not in protected:
            ending = "\n" if line.endswith("\n") else ""
            content = line.rstrip("\r\n")
            indent = re.match(r"[ \t]*", content)[0]
            prefix = " " * indentation[index] if index in indentation else indent
            lines[index - 1] = prefix + content[len(indent) :].rstrip() + ending
    result = "".join(lines)
    try:
        if ast.dump(ast.parse(result)) == ast.dump(original_tree):
            return result
    except SyntaxError:
        pass
    # Unusual layouts containing protected literals stay byte-for-byte intact.
    return source


def safe_folder_name(name: str) -> str:
    """Keep the title readable while respecting Windows folder-name rules."""
    name = re.sub(r'[<>:"/\\|?*\x00-\x1f]+', "-", name)
    name = re.sub(r"\s+", " ", name).strip(" .")
    name = (
        name.encode("utf-16-le", errors="ignore")[:200]
        .decode("utf-16-le", errors="ignore")
        .rstrip(" .")
    )
    name = name or "Untitled animation"
    device = name.split(".", 1)[0].rstrip().upper()
    if device in {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"} or re.fullmatch(
        r"(?:COM|LPT)[1-9\u00b9\u00b2\u00b3]", device
    ):
        name = "_" + name
    return name


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


class _JobLimits(ctypes.Structure):
    _fields_ = [
        ("process_time", ctypes.c_int64),
        ("job_time", ctypes.c_int64),
        ("flags", wintypes.DWORD),
        ("minimum_working_set", ctypes.c_size_t),
        ("maximum_working_set", ctypes.c_size_t),
        ("active_processes", wintypes.DWORD),
        ("affinity", ctypes.c_size_t),
        ("priority", wintypes.DWORD),
        ("scheduling", wintypes.DWORD),
    ]


class _JobIO(ctypes.Structure):
    _fields_ = [
        (name, ctypes.c_uint64)
        for name in (
            "read_operations",
            "write_operations",
            "other_operations",
            "read_bytes",
            "write_bytes",
            "other_bytes",
        )
    ]


class _ExtendedJobLimits(ctypes.Structure):
    _fields_ = [
        ("basic", _JobLimits),
        ("io", _JobIO),
        ("process_memory", ctypes.c_size_t),
        ("job_memory", ctypes.c_size_t),
        ("peak_process_memory", ctypes.c_size_t),
        ("peak_job_memory", ctypes.c_size_t),
    ]


class _JobAccounting(ctypes.Structure):
    _fields_ = [
        ("user_time", ctypes.c_int64),
        ("kernel_time", ctypes.c_int64),
        ("period_user_time", ctypes.c_int64),
        ("period_kernel_time", ctypes.c_int64),
        ("page_faults", wintypes.DWORD),
        ("total_processes", wintypes.DWORD),
        ("active_processes", wintypes.DWORD),
        ("terminated_processes", wintypes.DWORD),
    ]


class WindowsRenderJob:
    """Own descendants even after their worker parent exits."""

    def __init__(self, process):
        self.lock = threading.Lock()
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        for name, arguments, result in (
            ("CreateJobObjectW", [wintypes.LPVOID, wintypes.LPCWSTR], wintypes.HANDLE),
            (
                "SetInformationJobObject",
                [wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD],
                wintypes.BOOL,
            ),
            (
                "OpenProcess",
                [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD],
                wintypes.HANDLE,
            ),
            (
                "AssignProcessToJobObject",
                [wintypes.HANDLE, wintypes.HANDLE],
                wintypes.BOOL,
            ),
            ("TerminateJobObject", [wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
            (
                "QueryInformationJobObject",
                [
                    wintypes.HANDLE,
                    ctypes.c_int,
                    wintypes.LPVOID,
                    wintypes.DWORD,
                    ctypes.POINTER(wintypes.DWORD),
                ],
                wintypes.BOOL,
            ),
            ("CloseHandle", [wintypes.HANDLE], wintypes.BOOL),
        ):
            function = getattr(self.kernel, name)
            function.argtypes, function.restype = arguments, result
        # Null security attributes make this handle noninheritable.
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            limits = _ExtendedJobLimits()
            limits.basic.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if not self.kernel.SetInformationJobObject(
                self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)
            ):
                raise ctypes.WinError(ctypes.get_last_error())
            process_handle = self.kernel.OpenProcess(0x0101, False, process.pid)
            if not process_handle:
                raise ctypes.WinError(ctypes.get_last_error())
            try:
                if not self.kernel.AssignProcessToJobObject(
                    self.handle, process_handle
                ):
                    raise ctypes.WinError(ctypes.get_last_error())
            finally:
                self.kernel.CloseHandle(process_handle)
        except Exception:
            self.close()
            raise

    def terminate(self):
        with self.lock:
            if self.handle and not self.kernel.TerminateJobObject(self.handle, 1):
                raise ctypes.WinError(ctypes.get_last_error())
            deadline = time.monotonic() + 5
            while self._active_count():
                if time.monotonic() >= deadline:
                    raise subprocess.TimeoutExpired("render process tree", 5)
                time.sleep(0.01)

    def _active_count(self):
        if not self.handle:
            return 0
        accounting = _JobAccounting()
        if not self.kernel.QueryInformationJobObject(
            self.handle, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None
        ):
            raise ctypes.WinError(ctypes.get_last_error())
        return accounting.active_processes

    def active(self):
        with self.lock:
            return bool(self._active_count())

    def close(self):
        with self.lock:
            if self.handle:
                if not self.kernel.CloseHandle(self.handle):
                    raise ctypes.WinError(ctypes.get_last_error())
                self.handle = None


def attach_process_tree(process):
    if os.name == "nt":
        process._studio_job = WindowsRenderJob(process)


def release_process_tree(process):
    job = process.__dict__.get("_studio_job")
    if job is not None:
        job.close()


def stop_process(process: subprocess.Popen | None):
    if process is None:
        return
    job = process.__dict__.get("_studio_job")
    if job is not None:
        job.terminate()
        return
    if os.name == "nt":
        if process.poll() is not None:
            return
        subprocess.run(
            ["taskkill", "/PID", str(process.pid), "/T", "/F"],
            capture_output=True,
            creationflags=subprocess.CREATE_NO_WINDOW,
            timeout=5,
        )
    else:
        try:
            # A child may hold stdout open after the process-group leader exits.
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def start_process_monitor(process):
    def finish_children():
        try:
            process.wait()
            stop_process(process)
        except (OSError, subprocess.SubprocessError):
            logging.exception("Unable to stop remaining render helpers")
            try:
                release_process_tree(process)
            except OSError:
                logging.exception("Unable to close the render process job")

    monitor = threading.Thread(target=finish_children, daemon=True)
    monitor.start()
    return monitor


def cancel_active():
    with LOCK:
        if ACTIVE and ACTIVE in JOBS:
            JOBS[ACTIVE]["_cancel"] = True
        process = PROCESS
    stop_process(process)


def shutdown_render():
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
    try:
        yield
    finally:
        await asyncio.to_thread(shutdown_render)
        await asyncio.to_thread(UPDATES.shutdown)


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
    return (
        (WEB / "index.html")
        .read_text(encoding="utf-8")
        .replace("__STUDIO_TOKEN__", TOKEN)
    )


@app.get("/api/health")
def health():
    return {
        "manim": importlib.metadata.version("manim"),
        "python": sys.version.split()[0],
        "latex": bool(shutil.which("latex") and shutil.which("dvisvgm")),
        "active": ACTIVE,
        "assets": str(ROOT / "assets"),
        "outputs": str(OUTPUTS),
        "standalone": STANDALONE,
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
        render_options = []
        if "width" in job:
            resolution = f"{job['width']}x{job['height']}"
            fps = job["fps"]
            render_options = [
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
            PYTHONPATH=str(ROOT) + os.pathsep + os.environ.get("PYTHONPATH", ""),
        )
        env.pop("MANIM_STUDIO_CUSTOM_FPS", None)
        if "_requested_fps" in job:
            env["MANIM_STUDIO_CUSTOM_FPS"] = job["_requested_rate"]
        kwargs = (
            {"creationflags": subprocess.CREATE_NO_WINDOW}
            if os.name == "nt"
            else {"start_new_session": True}
        )
        with LOCK:
            if job.get("_cancel"):
                job.update(status="cancelled", stage="Render cancelled")
                return
            PROCESS = subprocess.Popen(
                command,
                cwd=ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stdin=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                **kwargs,
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
                log.write(clean + "\n")
                log.flush()
                with LOCK:
                    job["log"] = (job["log"] + clean + "\n")[-40_000:]
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
                    if match:
                        job["stage"] = f"Animation {int(match[1]) + 1} · {match[2]}%"
                    elif "Combining" in clean or "Writing" in clean:
                        job["stage"] = "Putting your video together…"
                    job["elapsed"] = round(time.monotonic() - started, 1)
            result = process.wait()
        with LOCK:
            if job.get("_cancel"):
                job.update(status="cancelled", stage="Render cancelled")
                return
        if result:
            raise RuntimeError(
                "Manim couldn't render this script. Copy the error details and ask your LLM to fix the script, then paste it again."
            )
        matches = [
            path
            for path in directory.rglob("video.mp4")
            if "partial_movie_files" not in path.parts
        ]
        if not matches:
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
            "This script uses Tex or MathTex. Install MiKTeX from miktex.org/download and restart Studio, or ask your LLM to use Text instead.",
        )
    with LOCK:
        if UPDATE_INSTALLING:
            raise HTTPException(
                409, "Studio is preparing an update. Wait for it to restart."
            )
        if ACTIVE:
            raise HTTPException(
                409,
                "A video is already rendering. Wait for it to finish or cancel it first.",
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
            "stage": "Preparing your script…",
            "log": "",
            "elapsed": 0,
        }
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
        if ACTIVE != job_id or JOBS[job_id]["status"] not in {"queued", "rendering"}:
            raise HTTPException(409, "This render is no longer running.")
        JOBS[job_id]["_cancel"] = True
        process = PROCESS
    stop_process(process)
    return {"ok": True}


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


@app.post("/api/open/{folder}")
def open_folder(folder: str):
    if folder not in {"outputs", "assets"}:
        raise HTTPException(404, "Folder not found.")
    path = OUTPUTS if folder == "outputs" else ROOT / "assets"
    path.mkdir(parents=True, exist_ok=True)
    if os.name == "nt":
        os.startfile(str(path))
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(path)])
    else:
        subprocess.Popen(["xdg-open", str(path)])
    return {"ok": True, "path": str(path)}


app.mount("/static", StaticFiles(directory=WEB), name="static")
