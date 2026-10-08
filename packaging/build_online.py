"""Build the small online installer after packaging/build.py prepares the app."""

import json
import shutil
import subprocess

from build import (
    ROOT,
    CACHE,
    DIST,
    BUNDLE,
    VERSION,
    TEX_COMMANDS,
    GUI_COMMANDS,
    payload_path_length,
)
from provenance import build_source_wheels, locked_requirements, write_checksums


def main():
    stage = CACHE / "online-stage"
    if stage.exists():
        if stage.resolve().parent != CACHE.resolve():
            raise RuntimeError("Unsafe stage path")
        shutil.rmtree(stage)
    (stage / "core").mkdir(parents=True)
    shutil.copytree(
        BUNDLE / "app",
        stage / "core" / "app",
        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"),
    )
    shutil.copytree(BUNDLE / "tools", stage / "core" / "tools")
    shutil.copytree(
        BUNDLE / "licenses",
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
        shutil.copy2(BUNDLE / name, stage / "core" / name)
    notice = stage / "core" / "THIRD-PARTY-NOTICES.txt"
    notice.write_text(
        notice.read_text(encoding="utf-8").replace(
            "matching source, build recipe and license in licenses/FFmpeg/",
            "build recipe and license in licenses/FFmpeg/; matching source in the release Third-Party-Sources asset",
        ),
        encoding="utf-8",
    )
    shutil.copy2(BUNDLE / "LICENSE", stage / "core" / "LICENSE")
    shutil.copy2(BUNDLE / "README.md", stage / "core" / "README.md")
    if (BUNDLE / "docs").is_dir():
        shutil.copytree(
            BUNDLE / "docs",
            stage / "core" / "docs",
            ignore=shutil.ignore_patterns("*.md"),
        )
    (stage / "core" / "READ ME.txt").write_text(
        f"MANIM STUDIO {VERSION}\n\nOpen Manim Studio from the Start menu or Manim Studio.exe.\n"
        "Setup downloaded and prepared all rendering tools. No commands are needed.\n"
        "Standard animations and equations now work offline.\n\n"
        "Paste a complete Manim script, pick a scene, then click Render animation.\n"
        "The small tour points to each button. Help & quick tour replays it.\n"
        "Your drafts, assets and renders are kept in your local app data:\n"
        "%LOCALAPPDATA%/ManimStudio/Data. They survive app upgrades and uninstalling.\n"
    )
    shutil.copy2(ROOT / "packaging" / "bootstrap.py", stage / "bootstrap.py")
    shutil.copy2(
        ROOT / "packaging" / "tex-native-lock.json", stage / "tex-native-lock.json"
    )
    downloads = [
        item
        for item in json.loads(
            (ROOT / "packaging" / "downloads.json").read_text(encoding="utf-8")
        )
        if not item["name"].startswith("ffmpeg")
    ]
    (stage / "downloads.json").write_text(
        json.dumps(downloads, indent=2) + "\n", encoding="utf-8"
    )
    (stage / "vc").mkdir()
    for file in (BUNDLE / "runtime").glob("*140*.dll"):
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
            "math_warning": "MiKTeX setup utility is pinned; its basic package repository is mutable and the installed snapshot is inventoried during setup.",
        }
    )
    provenance_path.write_text(
        json.dumps(provenance, indent=2) + "\n", encoding="utf-8"
    )
    (stage / "tex-commands.json").write_text(
        json.dumps({"keep": sorted(TEX_COMMANDS), "gui": sorted(GUI_COMMANDS)})
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
            str(ROOT / "packaging" / "online.iss"),
        ],
        check=True,
    )
    write_checksums(DIST, VERSION)
    print("Online installer:", DIST / f"Manim-Studio-{VERSION}-Setup-x64.exe")


if __name__ == "__main__":
    main()
