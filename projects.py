"""Project folders: each holds its script, narration settings and own assets."""

from __future__ import annotations

from datetime import datetime, timezone
import ast
from contextlib import ExitStack
import json
import math
import os
from pathlib import Path
import re
import shutil
import threading
import tempfile
import uuid
from zipfile import BadZipFile, ZipFile

PROJECT_FILE = "project.json"
SCRIPT_FILE = "script.py"
ASSETS = "assets"
NARRATION_FILE = "narration.txt"
STORYBOARD_FILE = "storyboard.md"
MAX_PACKAGE_BYTES = 512 * 1024**2
MAX_PACKAGE_FILES = 2000
MAX_SOURCE = 500_000
MAX_ASSETS_LISTED = 2000
LOCK = threading.RLock()
IMAGE = {".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"}
AUDIO = {".wav", ".mp3", ".ogg", ".flac", ".m4a", ".aac"}
VECTOR = {".svg"}
VIDEO = {".mp4", ".mov", ".webm", ".mkv"}
DEFAULT_NARRATION = {
    "enabled": False,
    "text": "",
    "voice": "af_heart",
    "speed": 1.0,
    "quality": "high",
}


class ProjectError(Exception):
    """A user-facing problem with a project request."""


def _now():
    return datetime.now(timezone.utc).isoformat()


def safe_name(name: str, fallback: str, limit: int = 160) -> str:
    """Readable names that are also valid Windows file and folder names."""
    name = re.sub(r'[<>:"/\\|?*\x00-\x1f]+', "-", name)
    name = re.sub(r"\s+", " ", name).strip(" .")
    name = (
        name.encode("utf-16-le", errors="ignore")[:limit]
        .decode("utf-16-le", errors="ignore")
        .rstrip(" .")
    ) or fallback
    device = name.split(".", 1)[0].rstrip().upper()
    if device in {"CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"} or re.fullmatch(
        r"(?:COM|LPT)[1-9¹²³]", device
    ):
        name = "_" + name
    return name


def _write_atomic(path: Path, text: str):
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(text, encoding="utf-8", newline="\n")
    temporary.replace(path)


def _contained(path: Path, parent: Path) -> bool:
    try:
        return path.resolve().is_relative_to(parent.resolve())
    except (OSError, RuntimeError):
        return False


def clean_narration(value) -> dict:
    result = dict(DEFAULT_NARRATION)
    if not isinstance(value, dict):
        return result
    if isinstance(value.get("enabled"), bool):
        result["enabled"] = value["enabled"]
    if isinstance(value.get("text"), str) and len(value["text"]) <= 100_000:
        result["text"] = value["text"]
    if isinstance(value.get("voice"), str) and re.fullmatch(
        r"[a-z]{2}_[a-z]+", value["voice"]
    ):
        result["voice"] = value["voice"]
    if (
        isinstance(value.get("speed"), (int, float))
        and not isinstance(value["speed"], bool)
        and 0.5 <= value["speed"] <= 2.0
    ):
        result["speed"] = float(value["speed"])
    if value.get("quality") in {"standard", "high"}:
        result["quality"] = value["quality"]
    return result


def _style_id(value) -> bool:
    return isinstance(value, str) and bool(
        re.fullmatch(r"[a-z0-9][a-z0-9-]{0,63}", value)
    )


def _read_metadata(folder: Path) -> dict | None:
    try:
        data = json.loads((folder / PROJECT_FILE).read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        return None
    if (
        not isinstance(data, dict)
        or not isinstance(data.get("id"), str)
        or not re.fullmatch(r"[0-9a-f]{32}", data["id"])
        or not isinstance(data.get("name"), str)
    ):
        return None
    narration = clean_narration(data.get("narration"))
    try:
        text = (folder / NARRATION_FILE).read_text(encoding="utf-8")
        if len(text) <= 100_000:
            narration["text"] = text
    except (OSError, UnicodeError):
        pass
    return {
        "id": data["id"],
        "name": data["name"][:100] or "Untitled project",
        "created": data.get("created") if isinstance(data.get("created"), str) else "",
        "updated": data.get("updated") if isinstance(data.get("updated"), str) else "",
        "narration": narration,
        "folder": folder.name,
        # The project's own watermark settings, or None to inherit them.
        "watermark": data["watermark"]
        if isinstance(data.get("watermark"), dict)
        else None,
        # House style id (styles.py); absent means the default style.
        **({"style": data["style"]} if _style_id(data.get("style")) else {}),
    }


def _folders(root: Path):
    if not root.is_dir():
        return []
    return [
        folder
        for folder in root.iterdir()
        if folder.is_dir()
        and not folder.is_symlink()
        and not folder.name.startswith(".")
        and (folder / PROJECT_FILE).is_file()
    ]


def list_projects(root: Path) -> list[dict]:
    with LOCK:
        projects = [
            project
            for folder in _folders(root)
            if (project := _read_metadata(folder)) is not None
        ]
    return sorted(projects, key=lambda item: item["updated"], reverse=True)


def folder(root: Path, project_id: str) -> Path:
    if not re.fullmatch(r"[0-9a-f]{32}", project_id or ""):
        raise KeyError(project_id)
    with LOCK:
        for candidate in _folders(root):
            metadata = _read_metadata(candidate)
            if metadata and metadata["id"] == project_id:
                return candidate
    raise KeyError(project_id)


def workspace_folder(root: Path, project_id: str, workspace: str | None = None) -> Path:
    parent = folder(root, project_id)
    if workspace is None:
        return parent
    videos = parent / "videos"
    path = folder(videos, workspace)
    if (
        videos.is_symlink()
        or not _contained(path, parent)
        or path.resolve().parent != videos.resolve()
    ):
        raise KeyError(workspace)
    return path


def create_video(root: Path, project_id: str, name: str) -> dict:
    parent = folder(root, project_id)
    videos = parent / "videos"
    if videos.is_symlink() or not _contained(videos, parent):
        raise ProjectError("This project's videos folder cannot be used.")
    video = create(videos, name, narration={"enabled": True, "quality": "high"})
    return {**video, "id": project_id, "workspace": video["id"], "folder": parent.name}


def assets_dir(root: Path, project_id: str, workspace: str | None = None) -> Path:
    path = workspace_folder(root, project_id, workspace) / ASSETS
    path.mkdir(exist_ok=True)
    return path


def read(root: Path, project_id: str, workspace: str | None = None) -> dict:
    if workspace is not None:
        path = workspace_folder(root, project_id, workspace)
        video = read(path.parent, workspace)
        return {
            **video,
            "id": project_id,
            "workspace": workspace,
            "folder": folder(root, project_id).name,
        }
    path = folder(root, project_id)
    metadata = _read_metadata(path)
    try:
        source = (path / SCRIPT_FILE).read_text(encoding="utf-8")
    except OSError:
        source = ""
    return {**metadata, "source": source}


def _save_metadata(path: Path, metadata: dict):
    """Write project.json only, leaving narration.txt untouched."""
    stored = {key: metadata[key] for key in ("id", "name", "created", "updated")}
    if _style_id(metadata.get("style")):
        stored["style"] = metadata["style"]
    stored["narration"] = {
        key: value for key, value in metadata["narration"].items() if key != "text"
    }
    if metadata.get("watermark") is not None:
        stored["watermark"] = metadata["watermark"]
    _write_atomic(path / PROJECT_FILE, json.dumps(stored, indent=2))


def _save(path: Path, metadata: dict):
    _write_atomic(path / NARRATION_FILE, metadata["narration"]["text"])
    _save_metadata(path, metadata)


def create(root: Path, name: str, source: str = "", narration=None, style=None) -> dict:
    name = (name or "").strip()[:100] or "Untitled project"
    if len(source) > MAX_SOURCE:
        raise ProjectError("Scripts must be smaller than 500 KB.")
    with LOCK:
        root.mkdir(parents=True, exist_ok=True)
        existing = {path.name.casefold() for path in root.iterdir()}
        stem = safe_name(name, "Untitled project")
        number = 1
        while True:
            candidate = stem if number == 1 else f"{stem} ({number})"
            if candidate.casefold() not in existing:
                try:
                    path = root / candidate
                    path.mkdir()
                    break
                except FileExistsError:
                    existing.add(candidate.casefold())
            number += 1
        (path / ASSETS).mkdir()
        metadata = {
            "id": uuid.uuid4().hex,
            "name": name,
            "created": _now(),
            "updated": _now(),
            "narration": clean_narration(narration),
            "folder": path.name,
        }
        if _style_id(style):
            metadata["style"] = style
        _write_atomic(path / SCRIPT_FILE, source)
        _save(path, metadata)
    return {**metadata, "source": source}


def update(
    root: Path,
    project_id: str,
    *,
    name=None,
    source=None,
    narration=None,
    workspace=None,
    style=None,
):
    """``style`` is a style id, or "" to follow the default (or parent) style."""
    if workspace is not None:
        path = workspace_folder(root, project_id, workspace)
        video = update(
            path.parent,
            workspace,
            name=name,
            source=source,
            narration=narration,
            style=style,
        )
        return {
            **video,
            "id": project_id,
            "workspace": workspace,
            "folder": folder(root, project_id).name,
        }
    with LOCK:
        path = folder(root, project_id)
        metadata = _read_metadata(path)
        if name is not None:
            metadata["name"] = name.strip()[:100] or metadata["name"]
        if narration is not None:
            metadata["narration"] = clean_narration(narration)
        if style is not None:
            if style and not _style_id(style):
                raise ProjectError("Choose a style from the Styles list.")
            metadata.pop("style", None)
            if style:
                metadata["style"] = style
        if source is not None:
            if len(source) > MAX_SOURCE:
                raise ProjectError("Scripts must be smaller than 500 KB.")
            _write_atomic(path / SCRIPT_FILE, source)
        metadata["updated"] = _now()
        # Leave narration.txt alone unless it changed (a rename or style change
        # must not look like a narration edit to a running video workflow).
        (_save if narration is not None else _save_metadata)(path, metadata)
    return metadata


def watermark_settings(root: Path, project_id: str, workspace: str | None = None):
    """(own, parent) watermark settings; a video workspace inherits its project's."""
    with LOCK:
        parent = _read_metadata(folder(root, project_id))
        if workspace is None:
            return parent["watermark"], None
        own = _read_metadata(workspace_folder(root, project_id, workspace))
        return own["watermark"], parent["watermark"]


def set_watermark(root: Path, project_id: str, workspace: str | None, value):
    """Store a project's (or video's) watermark override; None inherits again."""
    with LOCK:
        path = workspace_folder(root, project_id, workspace)
        metadata = _read_metadata(path)
        if metadata is None:
            raise KeyError(project_id)
        metadata["watermark"] = value
        metadata["updated"] = _now()
        # Only metadata changes: rewriting narration.txt would race an agent
        # editing it and look like a video edit to a running workflow.
        _save_metadata(path, metadata)
    return value


def delete(root: Path, project_id: str):
    with LOCK:
        path = folder(root, project_id)
        if not _contained(path, root) or path.resolve().parent != root.resolve():
            raise ProjectError("This project folder cannot be removed.")
        try:
            shutil.rmtree(path)
        except OSError as error:
            raise ProjectError(
                "Couldn’t delete this project. Close any open project files, then try again."
            ) from error
        return None


def ensure_default(root: Path, legacy_assets: Path, preferences: dict) -> None:
    """Move earlier single-workspace drafts into a first project, once."""
    with LOCK:
        marker = root / ".legacy-imported"
        if marker.is_file():
            return
        if list_projects(root):
            _write_atomic(marker, "1\n")
            return
        draft = preferences.get("manim-draft", "")[:MAX_SOURCE]
        legacy_files = (
            [
                item
                for item in legacy_assets.iterdir()
                if item.name != "README.txt" and not item.is_symlink()
            ]
            if legacy_assets.is_dir() and not legacy_assets.is_symlink()
            else []
        )
        if not draft.strip() and not legacy_files:
            return
        name = preferences.get("manim-name", "").strip() or "My first project"
        project = create(root, name, draft)
        target = root / project["folder"] / ASSETS
        if legacy_assets.is_dir() and not legacy_assets.is_symlink():
            for item in legacy_assets.iterdir():
                if item.name == "README.txt" or item.is_symlink():
                    continue
                try:
                    if item.is_dir():
                        shutil.copytree(item, target / item.name, symlinks=False)
                    else:
                        shutil.copy2(item, target / item.name)
                except OSError:
                    continue
        _write_atomic(marker, "1\n")


def asset_kind(path: Path) -> str:
    suffix = path.suffix.lower()
    if suffix in IMAGE:
        return "image"
    if suffix in VECTOR:
        return "vector"
    if suffix in AUDIO:
        return "audio"
    if suffix in VIDEO:
        return "video"
    return "file"


def list_assets(directory: Path) -> list[dict]:
    items = []
    if not directory.is_dir():
        return items
    for current, folders, files in os.walk(directory):
        folders[:] = sorted(
            name
            for name in folders
            if not name.startswith(".") and not (Path(current) / name).is_symlink()
        )
        for name in sorted(files):
            path = Path(current) / name
            if name.startswith(".") or name.endswith(".part") or path.is_symlink():
                continue
            try:
                size = path.stat().st_size
            except OSError:
                continue
            items.append(
                {
                    "path": path.relative_to(directory).as_posix(),
                    "size": size,
                    "kind": asset_kind(path),
                }
            )
            if len(items) >= MAX_ASSETS_LISTED:
                return items
    return items


def asset_file(directory: Path, relative: str) -> Path:
    parts = [part for part in relative.replace("\\", "/").split("/") if part]
    if not parts or any(part in {".", ".."} for part in parts):
        raise KeyError(relative)
    path = directory.joinpath(*parts)
    if not _contained(path, directory) or path.is_symlink():
        raise KeyError(relative)
    return path


def reserve_asset(directory: Path, filename: str) -> Path:
    """Pick a new, non-overwriting asset name for an upload."""
    name = safe_name(Path(filename.replace("\\", "/")).name, "asset")
    stem, suffix = Path(name).stem or "asset", Path(name).suffix
    directory.mkdir(parents=True, exist_ok=True)
    number = 1
    while True:
        candidate = directory / (name if number == 1 else f"{stem} ({number}){suffix}")
        partial = candidate.with_name(candidate.name + ".part")
        if not candidate.exists() and not partial.exists():
            return candidate
        number += 1


def package_path(value: str) -> str:
    """Reject unsafe or ambiguous Windows paths rather than changing asset names."""
    parts = value.replace("\\", "/").split("/")
    if not parts or any(
        not part
        or part in {".", ".."}
        or safe_name(part.lstrip("."), "") != part.lstrip(".")
        for part in parts
    ):
        raise ProjectError(
            f"Unsafe package path: {value[:120]}. Use relative file paths."
        )
    return "/".join(parts)


def _package_text(entry, limit=MAX_SOURCE):
    stream, size = entry
    if size > limit * 4:
        raise ProjectError(
            "A package text file is too large. Keep scripts under 500 KB."
        )
    stream.seek(0)
    try:
        text = stream.read(limit * 4 + 1).decode("utf-8-sig")
    except UnicodeError as error:
        raise ProjectError(
            "Save package text files as UTF-8 and import again."
        ) from error
    if len(text) > limit:
        raise ProjectError(
            "A package text file is too large. Shorten it and import again."
        )
    return text


def _package_narration(entries: dict, manifest: dict) -> dict:
    settings = manifest.get("narration", {})
    if not isinstance(settings, dict):
        raise ProjectError(
            "project.json narration must be an object with voice settings."
        )
    settings = dict(settings)
    if "narration.json" in entries:
        try:
            value = json.loads(_package_text(entries["narration.json"], 100_000))
        except (ValueError, RecursionError) as error:
            raise ProjectError(
                "Fix narration.json: it must contain valid JSON."
            ) from error
        if isinstance(value, dict):
            settings.update(value)
            value = value.get("segments")
        if isinstance(value, list):
            if len(value) > 400:
                raise ProjectError("Keep narration under 400 segments.")
            lines, previous = [], -1
            for segment in value:
                if not isinstance(segment, dict) or not isinstance(
                    segment.get("text"), str
                ):
                    raise ProjectError(
                        "Each narration segment needs text and a start time in seconds."
                    )
                start = segment.get("start")
                if (
                    isinstance(start, bool)
                    or not isinstance(start, (int, float))
                    or not math.isfinite(start)
                    or not 0 <= start <= 3600
                    or start < previous
                ):
                    raise ProjectError(
                        "Use increasing narration start times between 0 and 3600 seconds."
                    )
                previous = start
                stamp = f"{start:.3f}".rstrip("0").rstrip(".") or "0"
                lines.append(f"[{stamp}s] {segment['text'].strip()}")
            settings["text"] = "\n\n".join(lines)
        elif value is not None:
            raise ProjectError(
                "narration.json must contain voice settings or a list of timed segments."
            )
    for name in (NARRATION_FILE, "narration.md"):
        if name in entries and "text" not in settings:
            settings["text"] = _package_text(entries[name], 100_000)
    if (
        not isinstance(settings.get("text", ""), str)
        or len(settings.get("text", "")) > 100_000
    ):
        raise ProjectError("Keep narration text under 100,000 characters.")
    settings.setdefault("enabled", bool(settings.get("text", "").strip()))
    return clean_narration(settings)


def import_package(root: Path, files: list, name="Imported project") -> dict:
    """Import streams transactionally. Only code, narration and assets are retained."""
    if not files or len(files) > MAX_PACKAGE_FILES:
        raise ProjectError("Choose a package with 1–2000 files.")
    if sum(size for _, _, size in files) > MAX_PACKAGE_BYTES:
        raise ProjectError("Keep the unpacked package under 512 MB.")
    entries, seen = {}, set()
    for path, stream, size in files:
        path = package_path(path)
        if path.casefold() in seen:
            raise ProjectError(
                f"Duplicate package path: {path}. Keep one copy of each file."
            )
        seen.add(path.casefold())
        if not any(
            part.startswith(".") or part == "__MACOSX" for part in path.split("/")
        ):
            entries[path] = (stream, size)
    # Accept a ZIP/folder with one or more enclosing directory levels.
    while entries and all("/" in path for path in entries):
        parents = {path.split("/", 1)[0] for path in entries}
        if len(parents) != 1:
            break
        name = next(iter(parents))
        entries = {path.split("/", 1)[1]: value for path, value in entries.items()}
    manifest = {}
    if PROJECT_FILE in entries:
        try:
            manifest = json.loads(_package_text(entries[PROJECT_FILE], 100_000))
        except (ValueError, RecursionError) as error:
            raise ProjectError(
                "Fix project.json: it must contain valid JSON."
            ) from error
        if not isinstance(manifest, dict):
            raise ProjectError("project.json must be an object.")
    sources = {
        path: _package_text(value)
        for path, value in entries.items()
        if path.endswith(".py") and not path.startswith("assets/")
    }
    entrypoint = manifest.get("entrypoint")
    if entrypoint is not None:
        if not isinstance(entrypoint, str) or package_path(entrypoint) not in sources:
            raise ProjectError(
                "Set project.json entrypoint to the Manim Python file in this package."
            )
    else:
        candidates = []
        for path, source in sources.items():
            try:
                tree = ast.parse(source)
            except SyntaxError:
                continue
            if any(
                isinstance(node, ast.ClassDef)
                and any(
                    isinstance(child, ast.FunctionDef) and child.name == "construct"
                    for child in node.body
                )
                for node in tree.body
            ):
                candidates.append(path)
        entrypoint = next(
            (path for path in (SCRIPT_FILE, "main.py") if path in candidates), None
        )
        if entrypoint is None and len(candidates) == 1:
            entrypoint = candidates[0]
        if entrypoint is None:
            raise ProjectError(
                "Include one Manim scene script, or set its entrypoint in project.json."
            )
    if "/" in entrypoint:
        raise ProjectError("Place the main Manim script at the root of the package.")
    settings = _package_narration(entries, manifest)
    warnings = []
    if settings["enabled"] and any(
        re.fullmatch(
            r"assets/(?:audio/)?(?:narration|chapter_\d+)\.(?:wav|mp3|m4a|ogg)",
            path,
            re.I,
        )
        for path in entries
    ):
        settings["enabled"] = False
        warnings.append(
            "Recorded narration was found. Generated narration is off to avoid doubling the voice; the script can play the included audio."
        )
    # Keep local Python modules; omit assembly/install helpers and generated output.
    required = {entrypoint}
    pending = [entrypoint]
    while pending:
        current = pending.pop()
        try:
            tree = ast.parse(sources[current])
        except SyntaxError:
            continue
        modules = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                modules.update(alias.name for alias in node.names)
            elif isinstance(node, ast.ImportFrom):
                base = (
                    current.rsplit("/", 1)[0] + "/"
                    if "/" in current and node.level
                    else ""
                )
                if node.module:
                    modules.add(base + node.module.replace(".", "/"))
                modules.update(
                    base
                    + (node.module.replace(".", "/") + "/" if node.module else "")
                    + alias.name
                    for alias in node.names
                )
        for module in modules:
            module = module.replace(".", "/")
            parts = module.split("/")
            paths = [module + ".py"] + [
                "/".join(parts[:i]) + "/__init__.py" for i in range(1, len(parts) + 1)
            ]
            for path in paths:
                if path in sources and path not in required:
                    required.add(path)
                    pending.append(path)
    root.mkdir(parents=True, exist_ok=True)
    if entrypoint != SCRIPT_FILE and SCRIPT_FILE in required:
        raise ProjectError(
            "Rename the helper script.py so it does not conflict with the imported main script."
        )
    with tempfile.TemporaryDirectory(prefix=".import-", dir=root) as temporary:
        staging = Path(temporary)
        project = create(
            staging,
            str(manifest.get("name") or name),
            sources[entrypoint],
            settings,
            # Keep a house style id; unknown ids fall back to the default style.
            style=manifest.get("style"),
        )
        target = staging / project["folder"]
        for path, (stream, _) in entries.items():
            if (
                not path.startswith("assets/")
                and path != STORYBOARD_FILE
                and (path not in required or path == entrypoint)
            ):
                continue
            destination = target / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            stream.seek(0)
            with destination.open("wb") as output:
                shutil.copyfileobj(stream, output, length=1 << 20)
        with LOCK:
            stem, number = project["folder"], 1
            existing = {path.name.casefold() for path in root.iterdir()}
            while (
                candidate := stem if number == 1 else f"{stem} ({number})"
            ).casefold() in existing:
                number += 1
            target.rename(root / candidate)
            project["folder"] = candidate
    return {**project, "warnings": warnings}


def import_zip(root: Path, stream, name="Imported project") -> dict:
    try:
        with ZipFile(stream) as archive, ExitStack() as opened:
            files = []
            infos = archive.infolist()
            if len(infos) > MAX_PACKAGE_FILES * 2:
                raise ProjectError("Keep ZIP packages under 2000 files.")
            for item in infos:
                package_path(item.filename.rstrip("/"))
                if (item.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ProjectError(
                        "Use regular files in ZIP packages; symbolic links are unsupported."
                    )
                if not item.is_dir():
                    files.append(
                        (
                            item.filename,
                            opened.enter_context(archive.open(item)),
                            item.file_size,
                        )
                    )
            return import_package(root, files, name)
    except (BadZipFile, RuntimeError, NotImplementedError, EOFError) as error:
        raise ProjectError(
            "This ZIP could not be read. Use an unencrypted ZIP and import again."
        ) from error
