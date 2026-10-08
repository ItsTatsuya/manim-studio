"""Build an offline Windows bundle from the locked runtime dependencies.

uv sync --locked; uv run python packaging/build.py
Downloads are cached in .build-cache. Build prerequisites are documented in AGENTS.md.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import stat
import pefile
from zipfile import ZipFile, ZIP_DEFLATED
from provenance import (
    build_source_wheels,
    cached_runtime_inputs,
    digest,
    extract_zip,
    install_ffmpeg_tools,
    install_locked_runtime,
    locked_requirements,
    verify_math_snapshot,
    write_checksums,
)
from prepare_vc import install_vc_runtime
from prepare_native_notices import install_native_notices

ROOT = Path(__file__).resolve().parents[1]
CACHE = ROOT / ".build-cache"
DIST = ROOT / "dist"
BUNDLE = DIST / "Manim Studio Portable"
VERSION = "1.0.0"


def payload_path_length(directory):
    # Windows and Inno count UTF-16 code units, including directory separators.
    return max(
        (
            len(str(path.relative_to(directory)).encode("utf-16-le")) // 2
            + int(path.is_dir())
            for path in directory.rglob("*")
        ),
        default=0,
    )


# MiKTeX distributes hundreds of duplicate launchers for book/publishing tools.
# Retain every CLI engine plus the public aliases used for Manim typesetting.
TEX_COMMANDS = set(
    "miktex initexmf mpm latex pdflatex pdftex tex etex xelatex xetex lualatex luatex luahbtex luahblatex texlua texluac dvilualatex dviluatex dvisvgm dvips dvipdfmx xdvipdfmx dvipng kpsewhich findtexmf makefmt makebase makepk maketfm mktexpk mktextfm mktexmf mktexfmt mf mpost bibtex makeindex epstopdf repstopdf extractbb ebb updmap texhash fc-cache fc-list fc-match".split()
)
GUI_COMMANDS = {
    "miktex-console",
    "miktex-console_admin",
    "miktex-taskbar-icon",
    "miktex-update",
    "miktex-update_admin",
    "miktex-texworks",
}


def math_ignore(directory, names):
    path = Path(directory)
    skip = set()
    if path.name == "x64" and path.parent.name == "bin":
        for name in names:
            file = Path(name)
            if file.suffix.lower() == ".exe":
                keep = file.stem in TEX_COMMANDS or (
                    file.stem.startswith("miktex-") and file.stem not in GUI_COMMANDS
                )
                if not keep:
                    skip.add(name)
    return skip


def reset_component(name):
    target = BUNDLE / name
    # Never remove user files or an external target, including junctions.
    if (
        target.resolve().parent != BUNDLE.resolve()
        or BUNDLE.resolve().parent != DIST.resolve()
    ):
        raise RuntimeError(f"Unsafe bundle component: {target}")
    if target.exists():
        for file in target.rglob("*"):
            if file.is_file():
                file.chmod(file.stat().st_mode | stat.S_IWRITE)
        shutil.rmtree(target)


def verify_math_imports(directory):
    files = {file.name.lower(): file for file in directory.glob("*") if file.is_file()}
    original_names = {
        file.name.lower()
        for file in (
            CACHE / "math-runtime" / "texmfs" / "install" / "miktex" / "bin" / "x64"
        ).glob("*.dll")
    }
    sources = [file for file in files.values() if file.suffix.lower() == ".exe"]
    visited = set()
    while sources:
        file = sources.pop()
        if file.name.lower() in visited:
            continue
        visited.add(file.name.lower())
        with pefile.PE(str(file), fast_load=True) as binary:
            binary.parse_data_directories(
                directories=[
                    pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_IMPORT"],
                    pefile.DIRECTORY_ENTRY["IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT"],
                ]
            )
            imports = getattr(binary, "DIRECTORY_ENTRY_IMPORT", []) + getattr(
                binary, "DIRECTORY_ENTRY_DELAY_IMPORT", []
            )
            for entry in imports:
                dependency = entry.dll.decode().lower()
                if dependency in original_names and dependency not in files:
                    raise RuntimeError(
                        f"Missing engine dependency: {file.name} -> {dependency}"
                    )
                if dependency in files:
                    sources.append(files[dependency])


def copy_file(source, destination):
    target = Path(destination)
    if target.is_file():
        target.chmod(target.stat().st_mode | stat.S_IWRITE)
    return shutil.copy2(source, destination)


def copy_tree(source, destination):
    shutil.copytree(
        source,
        destination,
        dirs_exist_ok=True,
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "tests", ".git"),
        copy_function=copy_file,
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--no-archive", action="store_true")
    parser.add_argument("--no-installer", action="store_true")
    args = parser.parse_args()
    # Check source inputs before replacing any existing app/runtime component.
    inputs = cached_runtime_inputs()
    math_snapshot = verify_math_snapshot(CACHE / "math-runtime")
    requirements, built_wheels = build_source_wheels(
        CACHE / "verified-source-wheels", locked_requirements()
    )
    BUNDLE.mkdir(parents=True, exist_ok=True)
    obsolete = sorted(
        math_ignore(
            str(
                CACHE / "math-runtime" / "texmfs" / "install" / "miktex" / "bin" / "x64"
            ),
            [
                file.name
                for file in (
                    CACHE
                    / "math-runtime"
                    / "texmfs"
                    / "install"
                    / "miktex"
                    / "bin"
                    / "x64"
                ).glob("*.exe")
            ],
        )
    )
    (ROOT / "packaging" / "prune-previous.iss").write_text(
        "\n".join(
            f'Type: files; Name: "{{app}}\\math\\texmfs\\install\\miktex\\bin\\x64\\{name}"'
            for name in obsolete
        )
        + "\n",
        encoding="utf-8",
    )
    for name in ("app", "runtime", "math", "tools", "webview2", "licenses"):
        reset_component(name)
    runtime = BUNDLE / "runtime"
    runtime.mkdir(exist_ok=True)
    extract_zip(CACHE / "python-3.12.10-embed-amd64.zip", runtime)
    (runtime / "python312._pth").write_text(
        "python312.zip\n.\nLib/site-packages\n../app\nimport site\n", encoding="utf-8"
    )
    # The isolated _pth ignores system Python and user site packages; install the
    # production lock graph directly instead of copying an arbitrary developer venv.
    install_locked_runtime(runtime, requirements, CACHE / "verified-source-wheels")
    app = BUNDLE / "app"
    app.mkdir(exist_ok=True)
    for name in (
        "desktop.py",
        "studio.py",
        "runtime.py",
        "worker.py",
        "updates.py",
        "update_handoff.py",
        "manim.cfg",
    ):
        shutil.copy2(ROOT / name, app / name)
    copy_tree(ROOT / "studio_ui", app / "studio_ui")
    # Expand the hash-verified CAB afresh; an old extracted tree is not a trusted input.
    import tempfile

    with tempfile.TemporaryDirectory(prefix="webview2-build-", dir=CACHE) as folder:
        subprocess.run(
            [
                str(Path(os.environ["WINDIR"]) / "System32" / "expand.exe"),
                "-F:*",
                str(CACHE / "webview2-x64.cab"),
                folder,
            ],
            check=True,
            stdout=subprocess.DEVNULL,
        )
        browsers = list(
            Path(folder).glob("Microsoft.WebView2.FixedVersionRuntime.*.x64")
        )
        if len(browsers) != 1:
            raise RuntimeError("Expected one browser tree in the verified WebView2 CAB")
        copy_tree(browsers[0], BUNDLE / "webview2")
    shutil.copytree(CACHE / "math-runtime", BUNDLE / "math", ignore=math_ignore)
    verify_math_imports(
        BUNDLE / "math" / "texmfs" / "install" / "miktex" / "bin" / "x64"
    )
    ffmpeg = install_ffmpeg_tools(BUNDLE)
    # The licensed, pinned official redist supplies app-local native-wheel dependencies.
    vc = install_vc_runtime(BUNDLE, runtime)
    native_notices = install_native_notices(BUNDLE)
    (BUNDLE / "portable.mode").write_text(
        "User files are kept in UserData beside this app.\n", encoding="utf-8"
    )
    (BUNDLE / "standalone.json").write_text(
        json.dumps(
            {
                "version": VERSION,
                "python": "3.12.10",
                "manim": "0.21.0",
                "architecture": "x64",
            },
            indent=2,
        ),
        encoding="utf-8",
    )
    provenance = {
        "schema": 1,
        "version": VERSION,
        "uv_lock_sha256": digest(ROOT / "uv.lock"),
        "requirements_sha256": digest(ROOT / "packaging" / "requirements-runtime.txt"),
        "runtime_inputs": inputs,
        "source_built_wheels": built_wheels,
        "standalone_ffmpeg": ffmpeg,
        "visual_cpp_runtime": vc,
        "native_notices": native_notices,
        "math_snapshot_sha256": math_snapshot["inventory"]["tree_sha256"],
        "math_warning": math_snapshot["warning"],
    }
    (BUNDLE / "BUILD-PROVENANCE.json").write_text(
        json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
    )
    shutil.copy2(ROOT / "LICENSE", BUNDLE / "LICENSE")
    shutil.copy2(ROOT / "README.md", BUNDLE / "README.md")
    copy_tree(ROOT / "docs", BUNDLE / "docs")
    (BUNDLE / "READ ME.txt").write_text(
        f"MANIM STUDIO {VERSION}\n\nOpen Manim Studio.exe. Everything needed is included. No Python or commands.\n\nPortable edition: extract the WHOLE ZIP before opening the app. Your drafts,\nassets and renders live in UserData next to the executable. Keep this folder\nwhen you update the app. The installer edition saves them in your local app data.\n\nThe quick tour points to each button. You can dismiss it or use the app while\nit is open. Paste a Manim script, then click Render animation.\nText, equations, graphics, video preview, and audio tools are bundled.\n\nRequires 64-bit Windows 10 (2004+) or Windows 11.\n",
        encoding="utf-8",
    )
    notices = BUNDLE / "THIRD-PARTY-NOTICES.txt"
    notices.write_text(
        "Manim Studio is an aggregate of separately licensed components.\n\nPython: PSF license (runtime/LICENSE.txt).\nManim: MIT; other Python package licenses are in runtime/Lib/site-packages/*.dist-info.\nMicrosoft WebView2: Microsoft redistributable runtime terms, included with webview2.\nMiKTeX and included TeX packages: individual licenses distributed under math.\nPrivate FFmpeg audio tools: LGPL 2.1 or later; matching source, build recipe and license in licenses/FFmpeg/.\nPyAV separately bundles GPL-enabled FFmpeg codec libraries; matching codec sources and upstream recipes are in the release Third-Party-Sources asset.\nGhostscript 9.25: AGPLv3; matching source and MiKTeX build recipe are in the release Third-Party-Sources asset.\nHost Grotesk and Geist Mono fonts: SIL Open Font License, in app/studio_ui/fonts.\nMicrosoft Visual C++ runtime: Microsoft redistributable DLLs.\n\nThis app uses its own Python environment; it does not install a system Python.\n",
        encoding="utf-8",
    )
    with notices.open("a", encoding="utf-8") as notice_file:
        notice_file.write(
            "\nCodeMirror 6 and editor dependencies: MIT; app/studio_ui/vendor/EDITOR-LICENSES.txt.\n"
        )
    from PIL import Image, ImageDraw, ImageFont

    image = Image.new("RGBA", (256, 256), "#111113")
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(
        (12, 12, 244, 244), radius=50, fill="#1e2231", outline="#526187", width=3
    )
    font = ImageFont.truetype(
        str(Path(os.environ["WINDIR"]) / "Fonts" / "seguisb.ttf"), 190
    )
    draw.text((41, -4), "m", font=font, fill="#a7bbff")
    draw.ellipse((196, 173, 218, 195), fill="#a7bbff")
    icon = ROOT / "packaging" / "studio.ico"
    image.save(
        icon, sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )
    shutil.copy2(icon, app / "studio.ico")
    compiler = (
        Path(os.environ["WINDIR"])
        / "Microsoft.NET"
        / "Framework64"
        / "v4.0.30319"
        / "csc.exe"
    )
    subprocess.run(
        [
            str(compiler),
            "/nologo",
            "/target:winexe",
            "/platform:x64",
            "/optimize+",
            "/reference:System.Windows.Forms.dll",
            f"/win32icon:{icon}",
            f"/out:{BUNDLE / 'Manim Studio.exe'}",
            str(ROOT / "packaging" / "Launcher.cs"),
        ],
        check=True,
    )
    subprocess.run(
        [
            str(compiler),
            "/nologo",
            "/target:winexe",
            "/platform:x64",
            "/optimize+",
            "/reference:System.Windows.Forms.dll",
            "/reference:System.IO.Compression.dll",
            "/reference:System.IO.Compression.FileSystem.dll",
            "/reference:System.Runtime.Serialization.dll",
            "/reference:System.Web.Extensions.dll",
            f"/out:{app / 'Studio Update.exe'}",
            str(ROOT / "packaging" / "UpdateHelper.cs"),
        ],
        check=True,
    )
    print("Bundled executable and runtimes:", BUNDLE, flush=True)
    if not args.no_archive:
        output = DIST / f"Manim-Studio-{VERSION}-Portable-x64.zip"
        with ZipFile(output, "w", ZIP_DEFLATED, compresslevel=9) as archive:
            for file in BUNDLE.rglob("*"):
                if (
                    file.is_file()
                    and not {"UserData", "__pycache__"}.intersection(
                        file.relative_to(BUNDLE).parts
                    )
                    and file.suffix != ".pyc"
                ):
                    archive.write(file, Path("Manim Studio") / file.relative_to(BUNDLE))
        print("Portable ZIP:", output, flush=True)
    if not args.no_installer:
        subprocess.run(
            [
                str(CACHE / "inno" / "ISCC.exe"),
                f"/DBundleDir={BUNDLE}",
                f"/DPayloadPathLength={payload_path_length(BUNDLE)}",
                str(ROOT / "packaging" / "studio.iss"),
            ],
            check=True,
        )
    write_checksums(DIST, VERSION)


if __name__ == "__main__":
    main()
