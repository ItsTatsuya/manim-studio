"""Refresh app releases while preserving their existing compressed runtimes."""

import argparse
from copy import copy
import os
from pathlib import Path
import subprocess
import tempfile
from zipfile import ZipFile, ZIP_DEFLATED
from build import ROOT, BUNDLE, DIST, CACHE, VERSION, copy_tree, copy_file
from prepare_native_notices import install_native_notices
from provenance import digest
import json


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--no-installer", action="store_true")
    parser.add_argument(
        "--include-server",
        action="store_true",
        help="Also update the four app Python files using the existing locked runtimes",
    )
    args = parser.parse_args()
    native_notices = install_native_notices(BUNDLE)
    provenance_file = BUNDLE / "BUILD-PROVENANCE.json"
    provenance = json.loads(provenance_file.read_text(encoding="utf-8"))
    provenance["native_notices"] = native_notices
    provenance["app_sources"] = {
        name: digest(ROOT / name)
        for name in ("studio.py", "runtime.py", "worker.py", "desktop.py")
    }
    provenance_file.write_text(
        json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
    )
    for name in ("studio.py", "runtime.py", "worker.py", "desktop.py"):
        if args.include_server:
            continue
        if (ROOT / name).read_bytes() != (BUNDLE / "app" / name).read_bytes():
            raise RuntimeError(
                "This is not a UI-only update; use packaging/build.py: " + name
            )
    copy_tree(ROOT / "studio_ui", BUNDLE / "app" / "studio_ui")
    documentation = [ROOT / "LICENSE", ROOT / "README.md"]
    retired_docs = {
        "START-HERE.txt",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "THIRD-PARTY-NOTICES.md",
        "docs/PACKAGING.md",
        "docs/RELEASE-AUDIT.md",
    }
    for name in retired_docs:
        (BUNDLE / name).unlink(missing_ok=True)
    documentation.extend(file for file in (ROOT / "docs").rglob("*") if file.is_file())
    replacements = {
        "Manim Studio/" + file.relative_to(ROOT).as_posix(): file
        for file in documentation
    }
    replacements["Manim Studio/BUILD-PROVENANCE.json"] = provenance_file
    replacements.update(
        {
            "Manim Studio/" + file.relative_to(BUNDLE).as_posix(): file
            for file in (BUNDLE / "licenses").rglob("*")
            if file.is_file()
        }
    )
    for file in documentation:
        target = BUNDLE / file.relative_to(ROOT)
        target.parent.mkdir(parents=True, exist_ok=True)
        copy_file(file, target)
    if args.include_server:
        for name in ("studio.py", "runtime.py", "worker.py", "desktop.py"):
            copy_file(ROOT / name, BUNDLE / "app" / name)
            replacements["Manim Studio/app/" + name] = ROOT / name
    notices = BUNDLE / "THIRD-PARTY-NOTICES.txt"
    text = notices.read_text()
    if "CodeMirror" not in text:
        notices.write_text(
            text
            + "\nCodeMirror 6 and editor dependencies: MIT; app/studio_ui/vendor/EDITOR-LICENSES.txt.\n"
        )
    source = DIST / f"Manim-Studio-{VERSION}-Portable-x64.zip"
    if not source.exists():
        raise RuntimeError("Build the portable release first")
    handle, temporary = tempfile.mkstemp(suffix=".zip", prefix="ui-refresh-", dir=DIST)
    os.close(handle)
    prefix = "Manim Studio/app/studio_ui/"
    try:
        with (
            ZipFile(source) as old,
            ZipFile(temporary, "w", ZIP_DEFLATED, compresslevel=9) as new,
        ):
            entries = sorted(old.infolist(), key=lambda entry: entry.header_offset)
            for index, entry in enumerate(entries):
                if (
                    entry.filename.startswith(prefix)
                    or entry.filename in replacements
                    or entry.filename.removeprefix("Manim Studio/") in retired_docs
                    or entry.filename == "Manim Studio/THIRD-PARTY-NOTICES.txt"
                ):
                    continue
                end = (
                    entries[index + 1].header_offset
                    if index + 1 < len(entries)
                    else old.start_dir
                )
                length = end - entry.header_offset
                info = copy(entry)
                info.header_offset = new.fp.tell()
                old.fp.seek(entry.header_offset)
                while length:
                    chunk = old.fp.read(min(length, 1024 * 1024))
                    if not chunk:
                        raise RuntimeError("Truncated portable archive")
                    new.fp.write(chunk)
                    length -= len(chunk)
                new.filelist.append(info)
                new.NameToInfo[info.filename] = info
            new.start_dir = new.fp.tell()
            for file in (ROOT / "studio_ui").rglob("*"):
                if (
                    file.is_file()
                    and "__pycache__" not in file.parts
                    and file.suffix != ".pyc"
                ):
                    new.write(
                        file, prefix + file.relative_to(ROOT / "studio_ui").as_posix()
                    )
            new.write(notices, "Manim Studio/THIRD-PARTY-NOTICES.txt")
            for name, file in replacements.items():
                new.write(file, name)
        with ZipFile(temporary) as checked:
            if len(checked.namelist()) != len(set(checked.namelist())):
                raise RuntimeError("Portable archive contains duplicate entries")
            if checked.testzip() is not None:
                raise RuntimeError("Portable archive CRC verification failed")
            for file in (ROOT / "studio_ui").rglob("*"):
                if (
                    file.is_file()
                    and "__pycache__" not in file.parts
                    and file.suffix != ".pyc"
                ):
                    assert (
                        checked.read(
                            prefix + file.relative_to(ROOT / "studio_ui").as_posix()
                        )
                        == file.read_bytes()
                    )
            if args.include_server:
                assert (
                    checked.read("Manim Studio/app/studio.py")
                    == (ROOT / "studio.py").read_bytes()
                )
            for name, file in replacements.items():
                assert checked.read(name) == file.read_bytes()
        os.replace(temporary, source)
    finally:
        Path(temporary).unlink(missing_ok=True)
    print(
        "PASS: portable archive verified; runtimes preserved; app matches source",
        flush=True,
    )
    if not args.no_installer:
        subprocess.run(
            [
                str(CACHE / "inno" / "ISCC.exe"),
                f"/DBundleDir={BUNDLE}",
                str(ROOT / "packaging" / "studio.iss"),
            ],
            check=True,
        )
    subprocess.run(
        [
            os.environ.get("UV_BIN", "uv"),
            "run",
            "python",
            str(ROOT / "packaging" / "build_online.py"),
        ],
        check=True,
    )


if __name__ == "__main__":
    main()
