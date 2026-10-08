"""Installer-only runtime preparation, using the downloaded embedded Python."""

import argparse
import ctypes
import hashlib
import json
import os
import re
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import traceback
from zipfile import ZipFile


def extract_zip(archive, destination):
    destination.mkdir(parents=True, exist_ok=True)
    with ZipFile(archive) as source:
        for entry in source.infolist():
            if (
                not (destination / entry.filename)
                .resolve()
                .is_relative_to(destination.resolve())
            ):
                raise RuntimeError("Unsafe archive path")
        source.extractall(destination)


def inventory(directory):
    records = []
    for file in sorted(directory.rglob("*")):
        if file.is_file() and "__pycache__" not in file.parts and file.suffix != ".pyc":
            with file.open("rb") as source:
                actual = hashlib.file_digest(source, "sha256").hexdigest()
            records.append(
                {
                    "path": file.relative_to(directory).as_posix(),
                    "size": file.stat().st_size,
                    "sha256": actual,
                }
            )
    encoded = json.dumps(records, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return {
        "algorithm": "sha256",
        "tree_sha256": hashlib.sha256(encoded).hexdigest(),
        "files": records,
    }


def verify_math_engines(bin_dir, env, native_lock):
    """Reject a moving mirror whose native renderers exceed this release's sources."""
    versions = {}
    for name, pattern in (
        ("latex.exe", r"MiKTeX 26\.5(?: Portable)?\)"),
        ("dvisvgm.exe", r"(?m)^dvisvgm 3\.6\s*$"),
    ):
        result = subprocess.run(
            [str(bin_dir / name), "--version"],
            env=env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=subprocess.CREATE_NO_WINDOW,
            timeout=60,
            check=True,
        )
        if not re.search(pattern, result.stdout):
            raise RuntimeError(
                "The equation renderer changed upstream. Download a newer Manim Studio release; this release includes sources for MiKTeX 26.5, dvisvgm 3.6 and Ghostscript 9.25."
            )
        versions[name] = result.stdout.splitlines()[0]

    class GhostscriptRevision(ctypes.Structure):
        _fields_ = [
            ("product", ctypes.c_char_p),
            ("copyright", ctypes.c_char_p),
            ("revision", ctypes.c_long),
            ("revisiondate", ctypes.c_long),
        ]

    revision = GhostscriptRevision()
    ghostscript = ctypes.CDLL(str(bin_dir / "mgsdll64.dll"))
    query = ghostscript.gsapi_revision
    query.argtypes = [ctypes.POINTER(GhostscriptRevision), ctypes.c_int]
    query.restype = ctypes.c_int
    if (
        query(ctypes.byref(revision), ctypes.sizeof(revision))
        or revision.revision != 925
    ):
        raise RuntimeError(
            "Ghostscript changed upstream. Download a newer Manim Studio release to obtain its matching native sources."
        )
    versions["mgsdll64.dll"] = str(revision.revision)
    expected = json.loads(native_lock.read_text(encoding="utf-8"))["files"]
    actual_names = {
        file.name
        for file in bin_dir.iterdir()
        if file.is_file() and file.suffix.lower() in {".exe", ".dll"}
    }
    if actual_names != set(expected):
        raise RuntimeError(
            "The native equation renderer file set changed upstream. Download a newer Manim Studio release to obtain its matching sources."
        )
    for file in bin_dir.iterdir():
        if file.is_file() and file.suffix.lower() in {".exe", ".dll"}:
            with file.open("rb") as source:
                actual = hashlib.file_digest(source, "sha256").hexdigest()
            if expected.get(file.name) != actual:
                raise RuntimeError(
                    "The equation renderer changed upstream ("
                    + file.name
                    + "). Download a newer Manim Studio release to obtain its matching native sources."
                )
    return versions


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--downloads", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()
    downloads, destination = args.downloads.resolve(), args.destination.resolve()
    if destination.parent != downloads or destination.name != "prepared":
        raise RuntimeError(
            "Runtime preparation must stay in the installer temporary folder"
        )
    args.log.parent.mkdir(parents=True, exist_ok=True)
    with args.log.open("w", encoding="utf-8") as log:

        def say(message):
            print(message, flush=True)
            log.write(message + "\n")
            log.flush()

        def stage(percent, message):
            say(f"STAGE|{percent}|{message}")

        def command(argv, env=None):
            say("Running: " + str(argv[0]))
            with subprocess.Popen(
                [str(value) for value in argv],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                env=env,
                creationflags=subprocess.CREATE_NO_WINDOW,
            ) as child:
                tail = []
                for line in child.stdout:
                    line = line.rstrip()
                    log.write(line + "\n")
                    log.flush()
                    tail.append(line)
                    tail = tail[-12:]
                    if any(
                        term in line.lower()
                        for term in (
                            "downloading ",
                            "installing ",
                            "successfully installed",
                        )
                    ):
                        print(line[:220], flush=True)
                if child.wait():
                    raise RuntimeError("The setup tool failed:\n" + "\n".join(tail))

        try:
            stage(5, "Checking downloaded tools")
            for item in json.loads((downloads / "downloads.json").read_text()):
                with (downloads / item["name"]).open("rb") as source:
                    actual = hashlib.file_digest(source, "sha256").hexdigest()
                if actual != item["sha256"]:
                    raise RuntimeError("Download verification failed: " + item["name"])

            runtime = destination / "runtime"
            (runtime / "Lib" / "site-packages").mkdir(parents=True, exist_ok=True)
            (runtime / "python312._pth").write_text(
                "python312.zip\n.\nLib/site-packages\n../app\nimport site\n"
            )
            extract_zip(downloads / "pip.whl", downloads / "pip-package")
            stage(15, "Downloading and installing Manim libraries")
            command(
                [
                    runtime / "python.exe",
                    "-u",
                    Path(__file__),
                    "--pip",
                    downloads / "pip-package",
                    "install",
                    "--target",
                    runtime / "Lib" / "site-packages",
                    "--require-hashes",
                    "--only-binary=:all:",
                    "--find-links",
                    downloads / "wheels",
                    "--no-compile",
                    "--no-cache-dir",
                    "--disable-pip-version-check",
                    "--retries",
                    "5",
                    "--timeout",
                    "60",
                    "-r",
                    downloads / "requirements-online.txt",
                ]
            )

            stage(40, "Preparing the built-in browser")
            extracted = downloads / "browser-extracted"
            extracted.mkdir(exist_ok=True)
            command(
                [
                    Path(os.environ["WINDIR"]) / "System32" / "expand.exe",
                    "-F:*",
                    downloads / "webview2.cab",
                    extracted,
                ]
            )
            browser = next(
                extracted.glob("Microsoft.WebView2.FixedVersionRuntime.*.x64")
            )
            shutil.copytree(browser, destination / "webview2", dirs_exist_ok=True)

            stage(60, "Downloading equation and font packages")
            extract_zip(downloads / "miktex.zip", downloads / "miktex-setup")
            setup = next(
                (downloads / "miktex-setup").rglob("miktexsetup_standalone.exe")
            )
            repository = downloads / "miktex-repository"
            for attempt in range(3):
                try:
                    command(
                        [
                            setup,
                            "--package-set=basic",
                            f"--local-package-repository={repository}",
                            "download",
                        ]
                    )
                    break
                except RuntimeError:
                    if attempt == 2:
                        raise
                    say("Retrying equation package download...")
            stage(80, "Installing the private equation renderer")
            math = destination / "math"
            command(
                [
                    setup,
                    "--package-set=basic",
                    f"--local-package-repository={repository}",
                    f"--portable={math}",
                    "install",
                ]
            )
            bin_dir = math / "texmfs" / "install" / "miktex" / "bin" / "x64"
            env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"])
            command(
                [bin_dir / "initexmf.exe", "--set-config-value=[MPM]AutoInstall=1"], env
            )
            command([bin_dir / "initexmf.exe", "--dump=latex"], env)
            rules = json.loads((downloads / "tex-commands.json").read_text())
            for file in bin_dir.glob("*.exe"):
                keep = file.stem in rules["keep"] or (
                    file.stem.startswith("miktex-") and file.stem not in rules["gui"]
                )
                if not keep:
                    if not file.resolve().is_relative_to(math.resolve()):
                        raise RuntimeError("Unsafe math component path")
                    file.unlink()

            stage(92, "Recording installed tool versions and package snapshot")
            versions = verify_math_engines(
                bin_dir, env, downloads / "tex-native-lock.json"
            )
            provenance = {
                "schema": 1,
                "downloads": json.loads((downloads / "downloads.json").read_text()),
                "requirements_sha256": hashlib.sha256(
                    (downloads / "requirements-online.txt").read_bytes()
                ).hexdigest(),
                "math_warning": "MiKTeX basic packages were retrieved from a mutable upstream repository. This inventory records this installation; it is not a pinned reproducible repository snapshot.",
                "math_inventory": inventory(math),
                "verified_math_engines": versions,
            }
            (destination / "INSTALLED-RUNTIME-PROVENANCE.json").write_text(
                json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
            )

            stage(95, "Checking the render engine")
            command(
                [
                    runtime / "python.exe",
                    "-c",
                    'import manim, webview, av; assert manim.__version__ == "0.21.0"',
                ]
            )
            # The verified private audio tools ship in the installer's core files.
            command([bin_dir / "latex.exe", "--version"], env)
            command([bin_dir / "dvisvgm.exe", "--version"], env)
            stage(100, "All tools are ready. Installing Manim Studio...")
        except Exception as error:
            say("ERROR|" + str(error).replace("\n", " ")[:800])
            say(traceback.format_exc())
            raise


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--pip":
        sys.path.insert(0, sys.argv[2])
        sys.argv = ["pip"] + sys.argv[3:]
        runpy.run_module("pip", run_name="__main__")
    else:
        main()
