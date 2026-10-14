"""Build the LGPL-only Windows audio tools from authenticated pinned sources.

Run on Windows: python packaging/build_ffmpeg.py
Requires 7z (or the compiler archive's self-extractor), tar, and GnuPG.
The output archive contains replaceable shared FFmpeg DLLs, matching source,
licenses, this recipe, configuration, and a hash-bearing provenance manifest.
Manim encodes video through PyAV; these tools handle Pydub audio and ffprobe.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tarfile
import urllib.request
from zipfile import ZipFile, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".build-cache"
VERSION = "9.0.2"
SOURCE_URL = f"https://ffmpeg.org/releases/ffmpeg-{VERSION}.tar.xz"
SOURCE_SHA256 = "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
COMPILER_URL = "https://github.com/skeeto/w64devkit/releases/download/v2.10.0/w64devkit-x64-2.10.0.7z.exe"
COMPILER_SHA256 = "18d0a4c71a166f8401ab6305781bec5882b40b5e06ba9807c61cb5f3b3c6325e"
COMPILER_SOURCE_URL = (
    "https://github.com/skeeto/w64devkit/releases/download/v2.10.0/source.tar"
)
COMPILER_SOURCE_SHA256 = (
    "1e7a789bbc3ec58a2717b7a9069acec6a5abc58e38fa3a4dc801f22fc986e3a4"
)
SIGNING_KEY_URL = "https://ffmpeg.org/ffmpeg-devel.asc"
SIGNING_FINGERPRINT = "FCF986EA15E6E293A5644F10B4322F04D67658D8"
CONFIGURE = [
    "--prefix=../ffmpeg-install",
    "--target-os=mingw32",
    "--arch=x86_64",
    "--disable-autodetect",
    "--disable-gpl",
    "--disable-nonfree",
    "--disable-version3",
    "--disable-static",
    "--enable-shared",
    "--disable-asm",
    "--disable-debug",
    "--disable-doc",
    "--disable-network",
    "--disable-avdevice",
    "--disable-swscale",
    "--disable-everything",
    "--enable-ffmpeg",
    "--enable-ffprobe",
    "--disable-ffplay",
    "--enable-avcodec",
    "--enable-avformat",
    "--enable-avfilter",
    "--enable-swresample",
    "--enable-protocol=file,pipe",
    "--enable-demuxer=wav,mp3,aac,ogg,mov,matroska,flac,concat,pcm_s16le",
    "--enable-muxer=wav,adts,ogg,mp4,mov,matroska,flac,pcm_s16le",
    "--enable-parser=aac,aac_latm,mpegaudio,flac,opus,vorbis",
    "--enable-decoder=mp3,mp3float,mp3adu,mp3adufloat,mp3on4,mp3on4float,aac,aac_fixed,aac_latm,flac,vorbis,opus,pcm_s8,pcm_u8,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f64le,pcm_alaw,pcm_mulaw,wavpack",
    "--enable-encoder=pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,aac,flac,vorbis",
    "--enable-filter=abuffer,abuffersink,anull,aresample,aformat,atrim,volume,amix,afade,adelay",
    "--enable-bsf=aac_adtstoasc,extract_extradata",
    "--extra-cflags=-O2",
    "--extra-ldflags=-static-libgcc",
]


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def download(url: str, path: Path, digest: str | None = None) -> Path:
    if not path.is_file():
        temporary = path.with_suffix(path.suffix + ".partial")
        print(f"Downloading {url}", flush=True)
        with (
            urllib.request.urlopen(url, timeout=60) as response,
            temporary.open("wb") as output,
        ):
            shutil.copyfileobj(response, output)
        temporary.replace(path)
    if digest is not None and sha256(path) != digest:
        raise RuntimeError(
            f"SHA256 mismatch: {path.name}; remove the bad cache entry and retry"
        )
    return path


def run(command: list[str], *, cwd: Path | None = None, env: dict | None = None) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        encoding="utf-8",
        errors="replace",
    )
    return result.stdout


def fresh_directory(path: Path, parent: Path) -> None:
    if path.resolve().parent != parent.resolve() or parent.resolve() != CACHE.resolve():
        raise RuntimeError(f"Refusing to clean outside the build cache: {path}")
    if path.exists():
        shutil.rmtree(path)
    path.mkdir()


def verify_source(source: Path) -> str:
    signature = download(SOURCE_URL + ".asc", CACHE / (source.name + ".asc"))
    key = download(SIGNING_KEY_URL, CACHE / "ffmpeg-devel.asc")
    gpg = shutil.which("gpg")
    if not gpg:
        git_gpg = (
            Path(os.environ.get("ProgramFiles", "C:/Program Files"))
            / "Git/usr/bin/gpg.exe"
        )
        gpg = str(git_gpg) if git_gpg.is_file() else None
    if not gpg:
        raise RuntimeError(
            "GnuPG is required to authenticate the FFmpeg release signature"
        )
    keyring = CACHE / "ffmpeg-signing-keyring"
    keyring.mkdir(exist_ok=True)
    # Relative paths work with both native GnuPG and Git's MSYS GnuPG.
    base = [gpg, "--batch", "--homedir", keyring.name]
    run(base + ["--import", key.name], cwd=CACHE)
    status = run(
        base + ["--status-fd", "1", "--verify", signature.name, source.name], cwd=CACHE
    )
    if f"[GNUPG:] VALIDSIG {SIGNING_FINGERPRINT} " not in status:
        raise RuntimeError(
            "FFmpeg release signature did not match the pinned official signing key"
        )
    (CACHE / "ffmpeg-signature-verification.txt").write_text(status, encoding="utf-8")
    return status


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=min(os.cpu_count() or 2, 8))
    args = parser.parse_args()
    if os.name != "nt" or args.jobs < 1:
        parser.error("Build on Windows with a positive --jobs count")
    CACHE.mkdir(exist_ok=True)
    source = download(SOURCE_URL, CACHE / f"ffmpeg-{VERSION}.tar.xz", SOURCE_SHA256)
    verify_source(source)
    compiler_archive = download(
        COMPILER_URL, CACHE / "w64devkit-x64-2.10.0.7z.exe", COMPILER_SHA256
    )
    compiler_source = download(
        COMPILER_SOURCE_URL,
        CACHE / "w64devkit-2.10.0-source.tar",
        COMPILER_SOURCE_SHA256,
    )
    toolchain_parent = CACHE / "ffmpeg-toolchain"
    toolchain = toolchain_parent / "w64devkit"
    # Cached archives are authenticated; extracted trees are freshly recreated.
    fresh_directory(toolchain_parent, CACHE)
    sevenzip = shutil.which("7z") or shutil.which("7z.exe")
    if sevenzip:
        run([sevenzip, "x", str(compiler_archive), f"-o{toolchain_parent}", "-y"])
    else:
        run([str(compiler_archive), "-y", f"-o{toolchain_parent}"])
    source_parent = CACHE / "ffmpeg-source"
    source_dir = source_parent / f"ffmpeg-{VERSION}"
    fresh_directory(source_parent, CACHE)
    with tarfile.open(source) as archive:
        archive.extractall(source_parent, filter="data")
    env = dict(os.environ)
    # Use only the pinned build utilities, plus native Windows utilities.
    env["PATH"] = (
        str(toolchain / "bin") + os.pathsep + os.path.join(env["WINDIR"], "System32")
    )
    env.pop("CC", None)
    env.pop("CFLAGS", None)
    env.pop("LDFLAGS", None)
    (source_dir / "ffbuild/tmp").mkdir(parents=True, exist_ok=True)
    env["TMPDIR"] = "./ffbuild/tmp"
    shell = str(toolchain / "bin/sh.exe")
    build_commands = [
        "sh configure " + shlex.join(CONFIGURE),
        f"make V=1 -j{args.jobs}",
        "make V=1 install",
    ]
    with (CACHE / "ffmpeg-build.log").open("w", encoding="utf-8") as log:
        for command in build_commands:
            print(f"Building: {command.split()[0]}", flush=True)
            log.write(command + "\n")
            log.flush()
            subprocess.run(
                [shell, "-c", command],
                cwd=source_dir,
                env=env,
                check=True,
                stdout=log,
                stderr=subprocess.STDOUT,
            )
    install = source_parent / "ffmpeg-install"
    binaries = sorted((install / "bin").glob("*.exe")) + sorted(
        (install / "bin").glob("*.dll")
    )
    if not {"ffmpeg.exe", "ffprobe.exe"}.issubset({file.name for file in binaries}):
        raise RuntimeError("Build did not produce both required tools")
    version_text = run([str(install / "bin/ffmpeg.exe"), "-version"])
    license_text = run([str(install / "bin/ffmpeg.exe"), "-L"])
    if (
        "GNU Lesser General Public License" not in " ".join(license_text.split())
        or "--enable-gpl" in version_text
    ):
        raise RuntimeError(
            "Built FFmpeg did not report the intended LGPL configuration"
        )
    run([str(install / "bin/ffprobe.exe"), "-version"])
    imports = {}
    dll_names = {file.name.casefold() for file in binaries if file.suffix == ".dll"}
    system_dlls = {
        "kernel32.dll",
        "msvcrt.dll",
        "shell32.dll",
        "bcrypt.dll",
        "advapi32.dll",
        "user32.dll",
        "ole32.dll",
        "ws2_32.dll",
    }
    for file in binaries:
        details = run([str(toolchain / "bin/objdump.exe"), "-p", str(file)])
        imports[file.name] = re.findall(r"DLL Name:\s+(\S+)", details)
        if any(
            name.casefold() not in dll_names | system_dlls
            for name in imports[file.name]
        ):
            raise RuntimeError(
                f"Unexpected external DLL dependency: {file.name}: {imports[file.name]}"
            )
    provenance = {
        "archive": "ffmpeg-studio.zip",
        "version": VERSION,
        "license": "LGPL-2.1-or-later",
        "source_url": SOURCE_URL,
        "source_sha256": SOURCE_SHA256,
        "signature_url": SOURCE_URL + ".asc",
        "signing_key_url": SIGNING_KEY_URL,
        "verified_signing_fingerprint": SIGNING_FINGERPRINT,
        "compiler_url": COMPILER_URL,
        "compiler_sha256": COMPILER_SHA256,
        "compiler_source_url": COMPILER_SOURCE_URL,
        "compiler_source_sha256": COMPILER_SOURCE_SHA256,
        "compiler_version": "w64devkit 2.10.0",
        "recipe": "packaging/build_ffmpeg.py",
        "recipe_sha256": sha256(Path(__file__)),
        "configure": CONFIGURE,
        "shared_libraries": True,
        "external_libraries": [],
        "notes": "Audio tools only. MP3 is decode-only; native Vorbis encoding requires -strict experimental. Video encoding uses separately distributed PyAV.",
        "binaries": {file.name: sha256(file) for file in binaries},
        "dll_imports": imports,
    }
    output = CACHE / provenance["archive"]
    temporary = output.with_suffix(".zip.partial")
    with ZipFile(temporary, "w", ZIP_DEFLATED, compresslevel=9) as archive:
        for file in binaries:
            archive.write(file, f"tools/{file.name}")
        archive.writestr(
            "tools/ffmpeg-provenance.json", json.dumps(provenance, indent=2) + "\n"
        )
        for name in ("COPYING.LGPLv2.1", "LICENSE.md"):
            archive.write(source_dir / name, f"licenses/FFmpeg/{name}")
        archive.write(source, f"licenses/FFmpeg/{source.name}")
        archive.write(Path(__file__), "licenses/FFmpeg/build_ffmpeg.py")
        archive.write(source_dir / "ffbuild/config.mak", "licenses/FFmpeg/config.mak")
        archive.writestr("licenses/FFmpeg/version.txt", version_text)
        archive.writestr(
            "licenses/FFmpeg/signature-verification.txt",
            (CACHE / "ffmpeg-signature-verification.txt").read_text(encoding="utf-8"),
        )
        archive.write(
            CACHE / (source.name + ".asc"), f"licenses/FFmpeg/{source.name}.asc"
        )
        archive.write(CACHE / "ffmpeg-devel.asc", "licenses/FFmpeg/ffmpeg-devel.asc")
        archive.write(
            toolchain / "COPYING.MinGW-w64-runtime.txt",
            "licenses/FFmpeg/COPYING.MinGW-w64-runtime.txt",
        )
        # Include GCC's Runtime Library Exception and GPL text from the exact
        # compiler-source release used to produce the statically linked libgcc.
        with tarfile.open(compiler_source) as sources:
            gcc_member = next(
                member
                for member in sources.getmembers()
                if re.search(r"(?:^|/)gcc-[^/]+\.tar\.(?:xz|gz|bz2)$", member.name)
            )
            with tarfile.open(
                fileobj=sources.extractfile(gcc_member), mode="r|*"
            ) as gcc:
                found = set()
                for member in gcc:
                    if (
                        Path(member.name).name in {"COPYING.RUNTIME", "COPYING3"}
                        and member.isfile()
                    ):
                        filename = Path(member.name).name
                        if filename not in found:
                            archive.writestr(
                                f"licenses/FFmpeg/GCC-{filename}",
                                gcc.extractfile(member).read(),
                            )
                            found.add(filename)
                    if len(found) == 2:
                        break
                if found != {"COPYING.RUNTIME", "COPYING3"}:
                    raise RuntimeError(
                        "Compiler sources did not contain both GCC runtime license files"
                    )
    temporary.replace(output)
    provenance["sha256"] = sha256(output)
    (CACHE / "ffmpeg-studio.manifest.json").write_text(
        json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
    )
    print(f"Ready: {output} ({output.stat().st_size:,} bytes)", flush=True)


if __name__ == "__main__":
    main()
