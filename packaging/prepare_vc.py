"""Record and ship release CRT files from a licensed Visual Studio installation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import urllib.request

from provenance import CACHE, ROOT, digest, verify_file

NAMES = (
    "msvcp140.dll",
    "msvcp140_1.dll",
    "msvcp140_2.dll",
    "concrt140.dll",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "msvcp140_atomic_wait.dll",
    "msvcp140_codecvt_ids.dll",
)
LOCK = ROOT / "packaging" / "vc-runtime.json"
TERMS_URL = "https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx"


def capture(source: Path, visual_studio: Path) -> None:
    source = source.resolve()
    if (
        source.name != "Microsoft.VC143.CRT"
        or source.parent.name != "x64"
        or "debug_nonredist" in source.parts
    ):
        raise RuntimeError(
            "Select the standard x64 VC143 release redistributable directory"
        )
    if not (visual_studio / "Common7" / "IDE" / "devenv.exe").is_file():
        raise RuntimeError("A licensed Visual Studio installation is required")
    redist = visual_studio / "Licenses" / "1033" / "Redist.txt"
    if not redist.is_file():
        raise RuntimeError("Visual Studio redistribution terms are missing")
    target = CACHE / "vc-runtime"
    target.mkdir(exist_ok=True)
    for name in NAMES:
        shutil.copy2(source / name, target / name)
    shutil.copy2(redist, target / "REDIST.txt")
    # Preserve the original Microsoft agreement; never relicense the CRT as MIT.
    with urllib.request.urlopen(TERMS_URL, timeout=60) as response:
        (target / "Visual-Studio-Community-License.docx").write_bytes(response.read())
    records = {
        name: digest(target / name)
        for name in (*NAMES, "REDIST.txt", "Visual-Studio-Community-License.docx")
    }
    LOCK.write_text(
        json.dumps(
            {
                "schema": 1,
                "component": "Microsoft VC143 release CRT",
                "version": source.parent.parent.name,
                "origin": "Visual Studio 2022 release redistributable directory",
                "architecture": "x64",
                "terms_url": TERMS_URL,
                "redist_reference": "https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution",
                "files": records,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def install_vc_runtime(bundle: Path, runtime: Path) -> dict:
    manifest = json.loads(LOCK.read_text(encoding="utf-8"))
    if set(manifest.get("files", {})) != {
        *NAMES,
        "REDIST.txt",
        "Visual-Studio-Community-License.docx",
    }:
        raise RuntimeError(
            "CRT manifest must contain the complete reviewed release files and licenses"
        )
    licenses = bundle / "licenses" / "Microsoft-VC-Runtime"
    licenses.mkdir(parents=True, exist_ok=True)
    for name, expected in manifest["files"].items():
        if Path(name).name != name:
            raise RuntimeError("Unsafe CRT filename")
        file = verify_file(CACHE / "vc-runtime" / name, expected)
        shutil.copy2(file, (runtime if name in NAMES else licenses) / name)
    shutil.copy2(LOCK, licenses / "PROVENANCE.json")
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--visual-studio", type=Path, required=True)
    args = parser.parse_args()
    capture(args.source, args.visual_studio)
