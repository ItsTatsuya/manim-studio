"""Verified packaging inputs shared by the offline and online builders."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import ssl
import subprocess
import sys
import tempfile
import urllib.request
import urllib.error
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".build-cache"
SDISTS = (
    {
        "name": "srt",
        "version": "3.5.3",
        "filename": "srt-3.5.3.tar.gz",
        "url": "https://files.pythonhosted.org/packages/66/b7/4a1bc231e0681ebf339337b0cd05b91dc6a0d701fa852bb812e244b7a030/srt-3.5.3.tar.gz",
        "sha256": "4884315043a4f0740fd1f878ed6caa376ac06d70e135f306a6dc44632eed0cc0",
    },
    {
        "name": "proxy-tools",
        "version": "0.1.0",
        "filename": "proxy_tools-0.1.0.tar.gz",
        "url": "https://files.pythonhosted.org/packages/f2/cf/77d3e19b7fabd03895caca7857ef51e4c409e0ca6b37ee6e9f7daa50b642/proxy_tools-0.1.0.tar.gz",
        "sha256": "ccb3751f529c047e2d8a58440d86b205303cf0fe8146f784d1cbcd94f0a28010",
    },
)


def digest(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def verify_file(path: Path, expected: str) -> Path:
    if not re.fullmatch(r"[a-f0-9]{64}", expected):
        raise RuntimeError(f"Invalid pinned SHA256 for {path.name}")
    if digest(path) != expected:
        raise RuntimeError(f"Packaging input failed SHA256 verification: {path.name}")
    return path


def download_verified(item: dict, directory: Path) -> Path:
    """Reuse only verified archives; publish a newly fetched archive atomically."""
    filename = item.get("filename", item.get("name"))
    if not filename or Path(filename).name != filename:
        raise RuntimeError("Download filename must be a plain basename")
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / filename
    if target.exists():
        return verify_file(target, item["sha256"])
    request = urllib.request.Request(
        item["url"], headers={"User-Agent": "ManimStudio-Packaging/2.2"}
    )
    if not item["url"].startswith("https://"):
        raise RuntimeError("Packaging sources must use HTTPS")
    handle, temporary = tempfile.mkstemp(prefix="download-", dir=directory)
    try:
        with os.fdopen(handle, "wb") as output:
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    shutil.copyfileobj(response, output)
            except urllib.error.URLError as error:
                # Windows Schannel can complete a legitimate certificate chain
                # that Python's OpenSSL store cannot. Keep TLS checks enabled and
                # verify the pinned content hash regardless of download transport.
                curl = (
                    Path(os.environ.get("WINDIR", "C:/Windows"))
                    / "System32"
                    / "curl.exe"
                )
                if (
                    not isinstance(error.reason, ssl.SSLCertVerificationError)
                    or not curl.is_file()
                ):
                    raise
                output.close()
                subprocess.run(
                    [
                        str(curl),
                        "--fail",
                        "--silent",
                        "--show-error",
                        "--location",
                        "--proto",
                        "=https",
                        "--proto-redir",
                        "=https",
                        "--connect-timeout",
                        "20",
                        "--max-time",
                        "180",
                        "--output",
                        temporary,
                        item["url"],
                    ],
                    check=True,
                )
        verify_file(Path(temporary), item["sha256"])
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return target


def extract_zip(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    resolved = destination.resolve()
    with ZipFile(archive) as source:
        for entry in source.infolist():
            if not (destination / entry.filename).resolve().is_relative_to(resolved):
                raise RuntimeError("Unsafe packaging archive path")
        source.extractall(destination)


def locked_requirements() -> str:
    """Reject stale exported requirements without modifying uv.lock or the venv."""
    command = [
        os.environ.get("UV_BIN", "uv"),
        "export",
        "--locked",
        "--no-dev",
        "--no-emit-project",
        "--format",
        "requirements-txt",
        "--no-header",
        "--no-annotate",
    ]
    exported = subprocess.run(
        command, cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8"
    ).stdout
    checked_in = (ROOT / "packaging" / "requirements-runtime.txt").read_text(
        encoding="utf-8"
    )

    def normalized(text):
        return "\n".join(
            line.strip()
            for line in text.splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        )

    if normalized(exported) != normalized(checked_in):
        raise RuntimeError(
            "requirements-runtime.txt is stale; export it from the current uv.lock"
        )
    return exported


def build_source_wheels(destination: Path, requirements: str) -> tuple[str, list[dict]]:
    """Build the two source-only dependencies afresh from verified locked sdists."""
    destination.mkdir(parents=True, exist_ok=True)
    records = []
    for item in SDISTS:
        block = re.search(
            rf"^{re.escape(item['name'])}==.*?(?=^[a-zA-Z0-9]|\Z)",
            requirements,
            re.M | re.S,
        )
        if (
            not block
            or not block.group().startswith(f"{item['name']}=={item['version']}")
            or item["sha256"] not in block.group()
        ):
            raise RuntimeError(
                "Source-only dependency no longer matches uv.lock: " + item["name"]
            )
        source = download_verified(item, CACHE / "verified-sdists")
        # A private output directory prevents any wheel left by an earlier build
        # from becoming trusted merely because its current hash was calculated.
        with tempfile.TemporaryDirectory(prefix="wheel-build-", dir=CACHE) as folder:
            subprocess.run(
                [
                    os.environ.get("UV_BIN", "uv"),
                    "build",
                    "--wheel",
                    str(source),
                    "--no-cache",
                    "--no-build-logs",
                    "--python",
                    sys.executable,
                    "--out-dir",
                    folder,
                ],
                check=True,
                cwd=ROOT,
            )
            wheels = list(Path(folder).glob("*.whl"))
            if len(wheels) != 1:
                raise RuntimeError(
                    "Expected exactly one newly built wheel for " + item["name"]
                )
            wheel = wheels[0]
            expected_prefix = (
                item["name"].replace("-", "_") + "-" + item["version"] + "-"
            )
            if not wheel.name.startswith(expected_prefix):
                raise RuntimeError(
                    "Built wheel does not match the locked source package"
                )
            target = destination / wheel.name
            shutil.copy2(wheel, target)
        wheel_hash = digest(target)
        replacement = (
            f"{item['name']}=={item['version']} \\\n    --hash=sha256:{wheel_hash}\n"
        )
        requirements = (
            requirements[: block.start()] + replacement + requirements[block.end() :]
        )
        records.append(
            {
                **item,
                "wheel": target.name,
                "wheel_sha256": wheel_hash,
                "build_command": ["uv", "build", "--wheel", "--no-cache"],
                "build_isolation": True,
                "build_dependencies": "Resolved by uv in a fresh isolated environment; not byte-reproducible",
            }
        )
    return requirements, records


def install_locked_runtime(
    runtime: Path, requirements: str, source_wheels: Path
) -> None:
    """Install only the hash-locked production graph; never copy the ambient venv."""
    target = runtime / "Lib" / "site-packages"
    target.mkdir(parents=True, exist_ok=True)
    requirements_path = runtime / "requirements-runtime-built.txt"
    requirements_path.write_text(requirements, encoding="utf-8")
    subprocess.run(
        [
            os.environ.get("UV_BIN", "uv"),
            "pip",
            "install",
            "--python",
            str(runtime / "python.exe"),
            "--target",
            str(target),
            "--require-hashes",
            "--only-binary",
            ":all:",
            "--no-deps",
            "--no-cache",
            "--link-mode",
            "copy",
            "--find-links",
            str(source_wheels),
            "-r",
            str(requirements_path),
        ],
        check=True,
        cwd=ROOT,
    )
    forbidden = {"ruff", "playwright", "pefile", "pyee", "greenlet"}
    installed = {
        entry.name.split("-")[0].replace("_", "-").lower()
        for entry in target.glob("*.dist-info")
    }
    if installed.intersection(forbidden):
        raise RuntimeError("Development dependencies leaked into the packaged runtime")


def tree_inventory(directory: Path) -> dict:
    records = [
        {
            "path": file.relative_to(directory).as_posix(),
            "size": file.stat().st_size,
            "sha256": digest(file),
        }
        for file in sorted(directory.rglob("*"))
        if file.is_file() and "__pycache__" not in file.parts and file.suffix != ".pyc"
    ]
    encoded = json.dumps(records, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "algorithm": "sha256",
        "tree_sha256": hashlib.sha256(encoded).hexdigest(),
        "files": records,
    }


def verify_math_snapshot(directory: Path) -> dict:
    expected = json.loads(
        (ROOT / "packaging" / "math-snapshot.json").read_text(encoding="utf-8")
    )
    actual = tree_inventory(directory)
    if actual["tree_sha256"] != expected["inventory"]["tree_sha256"]:
        raise RuntimeError(
            "Cached MiKTeX snapshot changed; explicitly refresh packaging/math-snapshot.json after source review"
        )
    return expected


def write_checksums(dist: Path, version: str) -> None:
    files = sorted(
        file
        for file in dist.glob(f"Manim-Studio-{version}-*")
        if file.is_file() and file.suffix in {".exe", ".zip"}
    )
    (dist / "SHA256SUMS.txt").write_text(
        "".join(f"{digest(file)}  {file.name}\n" for file in files), encoding="utf-8"
    )


def cached_runtime_inputs() -> dict:
    items = {
        item["name"]: item
        for item in json.loads(
            (ROOT / "packaging" / "downloads.json").read_text(encoding="utf-8")
        )
    }
    files = {
        "python.zip": CACHE / "python-3.12.10-embed-amd64.zip",
        "webview2.cab": CACHE / "webview2-x64.cab",
    }
    for name, path in files.items():
        verify_file(path, items[name]["sha256"])
    return {
        name: {**items[name], "cache_filename": path.name}
        for name, path in files.items()
    }


def install_ffmpeg_tools(bundle: Path) -> dict:
    path = CACHE / "ffmpeg-studio.manifest.json"
    if not path.is_file():
        raise RuntimeError(
            "Build the verified private FFmpeg tools with packaging/build_ffmpeg.py first"
        )
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if manifest.get("archive") != "ffmpeg-studio.zip":
        raise RuntimeError("Unexpected private FFmpeg archive name")
    archive = verify_file(CACHE / manifest["archive"], manifest["sha256"])
    if (
        manifest.get("source_sha256")
        != "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
    ):
        raise RuntimeError(
            "Private FFmpeg source no longer matches the reviewed version"
        )
    with ZipFile(archive) as source:
        if any(
            entry.filename.split("/", 1)[0] not in {"tools", "licenses"}
            for entry in source.infolist()
        ):
            raise RuntimeError("Unexpected component in private FFmpeg archive")
    extract_zip(archive, bundle)
    for name, expected in manifest["binaries"].items():
        if Path(name).name != name:
            raise RuntimeError("Unsafe FFmpeg binary name")
        verify_file(bundle / "tools" / name, expected)
    return manifest
