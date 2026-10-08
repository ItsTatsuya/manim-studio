"""Build the small online installer after packaging/build.py prepares the app."""

import json
import argparse
from pathlib import Path
import shutil
import subprocess
from zipfile import ZipFile, ZIP_DEFLATED

from build import (
    ROOT,
    CACHE,
    DIST,
    VERSION,
    payload_path_length,
)
from provenance import build_source_wheels, locked_requirements, write_checksums


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, default=DIST)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    if not output.is_relative_to(DIST.resolve()):
        raise RuntimeError("Online build output must stay within dist")
    bundle = output / "Manim Studio Portable"
    metadata = json.loads((bundle / "standalone.json").read_text(encoding="utf-8"))
    if metadata.get("webview2") != "evergreen":
        raise RuntimeError("Online installer requires an Evergreen bundle")
    stage = CACHE / "online-stage" / output.relative_to(DIST.resolve())
    if stage == CACHE / "online-stage":
        stage = stage / "default"
    if stage.exists():
        if not stage.resolve().is_relative_to((CACHE / "online-stage").resolve()):
            raise RuntimeError("Unsafe stage path")
        shutil.rmtree(stage)
    (stage / "core").mkdir(parents=True)
    shutil.copytree(
        bundle / "app",
        stage / "core" / "app",
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
    )
    shutil.copytree(bundle / "tools", stage / "core" / "tools")
    shutil.copytree(
        bundle / "licenses",
        stage / "core" / "licenses",
        ignore=shutil.ignore_patterns("ffmpeg-9.0.2.tar.xz"),
    )
    (stage / "core" / "licenses" / "FFmpeg" / "SOURCE-DOWNLOAD.txt").write_text(
        f"Matching source is provided alongside this installer:\nhttps://github.com/ItsTatsuya/manim-studio/releases/download/v{VERSION}/Manim-Studio-{VERSION}-Third-Party-Sources.zip\n",
        encoding="utf-8",
    )
    for name in (
        "Manim Studio.exe",
        "standalone.json",
        "THIRD-PARTY-NOTICES.txt",
        "BUILD-PROVENANCE.json",
    ):
        shutil.copy2(bundle / name, stage / "core" / name)
    notice = stage / "core" / "THIRD-PARTY-NOTICES.txt"
    notice.write_text(
        notice.read_text(encoding="utf-8").replace(
            "matching source, build recipe and license in licenses/FFmpeg/",
            "build recipe and license in licenses/FFmpeg/; matching source in the release Third-Party-Sources asset",
        ),
        encoding="utf-8",
    )
    shutil.copy2(bundle / "LICENSE", stage / "core" / "LICENSE")
    shutil.copy2(bundle / "README.md", stage / "core" / "README.md")
    if (bundle / "docs").is_dir():
        shutil.copytree(
            bundle / "docs",
            stage / "core" / "docs",
            ignore=shutil.ignore_patterns("*.md"),
        )
    (stage / "core" / "READ ME.txt").write_text(
        f"MANIM STUDIO {VERSION}\n\nOpen Manim Studio from the Start menu or Manim Studio.exe.\n"
        "Setup downloaded and prepared all rendering tools. No commands are needed.\n"
        + (
            "Standard animations and Typst equations work offline; Tex/MathTex need an external LaTeX installation.\n\n"
            if metadata["math"] == "none"
            else "Standard animations, LaTeX and Typst equations now work offline.\n\n"
        )
        + "Paste a complete Manim script, pick a scene, then click Render animation.\n"
        "The small tour points to each button. Help & quick tour replays it.\n"
        "Your drafts, assets and renders are kept in your local app data:\n"
        "%LOCALAPPDATA%/ManimStudio/Data. They survive app upgrades and uninstalling.\n"
    )
    shutil.copy2(ROOT / "packaging" / "bootstrap.py", stage / "bootstrap.py")
    (stage / "runtime-profile.json").write_text(
        json.dumps(metadata) + "\n", encoding="utf-8"
    )
    for name in ("tex-native-lock.json", "math-inventory.json"):
        original = bundle / "app" / name
        if original.is_file():
            shutil.copy2(original, stage / name)
        else:
            (stage / name).write_text("{}\n", encoding="utf-8")
    with ZipFile(
        stage / "math-runtime.zip", "w", ZIP_DEFLATED, compresslevel=9
    ) as archive:
        if (bundle / "math").is_dir():
            for file in (bundle / "math").rglob("*"):
                if (
                    file.is_file()
                    and "__pycache__" not in file.parts
                    and file.suffix != ".pyc"
                ):
                    archive.write(file, file.relative_to(bundle / "math"))
    shutil.copy2(
        bundle / "tools/MicrosoftEdgeWebview2Setup.exe",
        stage / "MicrosoftEdgeWebview2Setup.exe",
    )
    downloads = [
        item
        for item in json.loads(
            (ROOT / "packaging" / "downloads.json").read_text(encoding="utf-8")
        )
        if item["name"] in {"python.zip", "pip.whl"}
    ]
    (stage / "downloads.json").write_text(
        json.dumps(downloads, indent=2) + "\n", encoding="utf-8"
    )
    (stage / "vc").mkdir()
    for file in (bundle / "runtime").glob("*140*.dll"):
        shutil.copy2(file, stage / "vc" / file.name)
    (stage / "wheels").mkdir()
    # These two small pure-Python packages publish only source archives. Build
    # their locked sources here so users never need pip build tools or a compiler.
    requirements, built_wheels = build_source_wheels(
        stage / "wheels", locked_requirements()
    )
    (stage / "requirements-online.txt").write_text(requirements, encoding="utf-8")
    provenance_path = stage / "core" / "BUILD-PROVENANCE.json"
    provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
    provenance.update(
        {
            "edition": "online",
            "source_built_wheels": built_wheels,
            "math_warning": "The selected math payload is embedded from the verified offline snapshot and checked against its complete inventory during setup.",
        }
    )
    provenance_path.write_text(
        json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
    )
    (stage / "downloads.iss").write_text(
        "\n".join(
            f"DownloadPage.Add('{item['url']}', '{item['name']}', '{item['sha256']}');"
            for item in downloads
        )
    )
    subprocess.run(
        [
            str(CACHE / "inno" / "ISCC.exe"),
            f"/DStageDir={stage}",
            f"/DPayloadPathLength={payload_path_length(stage / 'core')}",
            f"/DOutputDir={output}",
            f"/DAppVersion={VERSION}",
            f"/DPruneRuntime={output / 'prune-runtime.iss'}",
            str(ROOT / "packaging" / "online.iss"),
        ],
        check=True,
    )
    write_checksums(output, VERSION)
    print("Online installer:", output / f"Manim-Studio-{VERSION}-Setup-x64.exe")


if __name__ == "__main__":
    main()
