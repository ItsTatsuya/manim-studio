"""Keep shipped code read-only and user files portable or per-user."""

import os
import json
import logging
from pathlib import Path
import shutil
import sys
import tomllib
from updates import stable_version, UpdateError

APP_ROOT = Path(__file__).resolve().parent
PACKAGE_ROOT = APP_ROOT.parent
STANDALONE = (PACKAGE_ROOT / "standalone.json").is_file()
PORTABLE = STANDALONE and (PACKAGE_ROOT / "portable.mode").is_file()


def app_version():
    try:
        version = (
            json.loads((PACKAGE_ROOT / "standalone.json").read_bytes())["version"]
            if STANDALONE
            else tomllib.loads(
                (APP_ROOT / "pyproject.toml").read_text(encoding="utf-8")
            )["project"]["version"]
        )
        stable_version(version)
        return version
    except (OSError, ValueError, KeyError, TypeError, RecursionError, UpdateError):
        logging.warning(
            "Unable to read Studio version metadata; using the source release version"
        )
    return "1.1.0"


APP_VERSION = app_version()
if os.environ.get("MANIM_STUDIO_DATA"):
    DATA_ROOT = Path(os.environ["MANIM_STUDIO_DATA"]).resolve()
elif STANDALONE:
    DATA_ROOT = (
        PACKAGE_ROOT / "UserData"
        if PORTABLE
        else Path(os.environ.get("LOCALAPPDATA", str(Path.home())))
        / "ManimStudio"
        / "Data"
    )
else:
    DATA_ROOT = APP_ROOT


def initialize():
    DATA_ROOT.mkdir(parents=True, exist_ok=True)
    (DATA_ROOT / "assets").mkdir(exist_ok=True)
    if STANDALONE:
        for name in ("manim.cfg",):
            if not (DATA_ROOT / name).exists():
                shutil.copy2(APP_ROOT / name, DATA_ROOT / name)
        paths = [
            PACKAGE_ROOT / "tools",
            PACKAGE_ROOT / "math" / "texmfs" / "install" / "miktex" / "bin" / "x64",
        ]
        os.environ["PATH"] = (
            os.pathsep.join(str(path) for path in paths if path.exists())
            + os.pathsep
            + os.environ.get("PATH", "")
        )
        # Never inherit an unrelated Python installation or virtual environment.
        for key in ("PYTHONHOME", "PYTHONPATH", "VIRTUAL_ENV"):
            os.environ.pop(key, None)


def renderer_python():
    executable = Path(sys.executable)
    console = executable.with_name("python.exe")
    return str(console if console.is_file() else executable)


initialize()
