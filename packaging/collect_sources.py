"""Collect pinned native component sources without executing upstream build code.

Initial reviewed catalog: python packaging/collect_sources.py --lock
Release archive: python packaging/collect_sources.py
The manifest describes coverage; it is not a written source offer.
"""

from __future__ import annotations

import argparse
import ast
import configparser
import json
import hashlib
from pathlib import Path
import shutil
import tarfile
import tempfile
import urllib.request
from zipfile import ZipFile, ZIP_STORED

from provenance import (
    ROOT,
    CACHE,
    digest,
    download_verified,
    tree_inventory,
    write_checksums,
)

VERSION = "1.0.0"
LOCK = ROOT / "packaging" / "source-lock.json"
SOURCE_CACHE = CACHE / "third-party-sources"
ANCHORS = [
    {
        "component": "pyav-ffmpeg-build",
        "version": "9.0.2-1",
        "commit": "bac889417e26be29615cb6a3f38318151fed55cd",
        "filename": "pyav-ffmpeg-bac889417e26be29615cb6a3f38318151fed55cd.tar.gz",
        "url": "https://codeload.github.com/PyAV-Org/pyav-ffmpeg/tar.gz/bac889417e26be29615cb6a3f38318151fed55cd",
    },
    {
        "component": "miktex",
        "version": "26.5",
        "commit": "77333fa300a9675ac3fdba45528a05bc3bff975d",
        "filename": "miktex-77333fa300a9675ac3fdba45528a05bc3bff975d.tar.gz",
        "url": "https://codeload.github.com/MiKTeX/miktex/tar.gz/77333fa300a9675ac3fdba45528a05bc3bff975d",
    },
    {
        "component": "ghostpdl",
        "version": "9.25",
        "filename": "ghostpdl-9.25.tar.gz",
        "url": "https://github.com/ArtifexSoftware/ghostpdl-downloads/releases/download/gs925/ghostpdl-9.25.tar.gz",
        "sha256": "784531e81f144c0f703a0f079e6fd1522eabb97b3a0670a92bdd1f39bf201d99",
    },
    {
        "component": "w64devkit-source",
        "version": "2.10.0",
        "filename": "w64devkit-2.10.0-source.tar",
        "url": "https://github.com/skeeto/w64devkit/releases/download/v2.10.0/source.tar",
        "sha256": "1e7a789bbc3ec58a2717b7a9069acec6a5abc58e38fa3a4dc801f22fc986e3a4",
        "hash_origin": "Upstream GitHub release asset SHA256",
        "scope": "Build toolchain sources, including GCC/MinGW runtime licensing",
    },
    {
        "component": "pango-build",
        "version": "1.58.2",
        "commit": "0dce9785ea638733764aebbd60b134387d8b44d8",
        "filename": "pango-build-0dce9785ea638733764aebbd60b134387d8b44d8.tar.gz",
        "url": "https://codeload.github.com/naveen521kk/pango-build/tar.gz/0dce9785ea638733764aebbd60b134387d8b44d8",
    },
    {
        "component": "cairo-win-build",
        "version": "1.18.6",
        "commit": "3a9a75de8f07b46430f9ccf477950f252790b8f4",
        "filename": "cairo-win-build-3a9a75de8f07b46430f9ccf477950f252790b8f4.tar.gz",
        "url": "https://codeload.github.com/pygobject/cairo-win-build/tar.gz/3a9a75de8f07b46430f9ccf477950f252790b8f4",
    },
    {
        "component": "manimpango-build",
        "version": "0.7.0",
        "commit": "035a36336974aa13fbc26d288b23de6c38e942f1",
        "filename": "manimpango-build-035a36336974aa13fbc26d288b23de6c38e942f1.tar.gz",
        "url": "https://codeload.github.com/ManimCommunity/ManimPango/tar.gz/035a36336974aa13fbc26d288b23de6c38e942f1",
        "scope": "Original Windows wheel CI and static native download/relinking recipe, omitted from PyPI sdist",
    },
    {
        "component": "gvdb",
        "commit": "2b42fc75f09dbe1cd1057580b5782b08f2dcb400",
        "filename": "gvdb-2b42fc75f09dbe1cd1057580b5782b08f2dcb400.tar.gz",
        "url": "https://codeload.github.com/GNOME/gvdb/tar.gz/2b42fc75f09dbe1cd1057580b5782b08f2dcb400",
        "scope": "GNOME official mirror of the exact GLib vendored GVDB revision",
    },
    *[
        {
            "component": component,
            "version": version,
            "filename": filename,
            "url": "https://repo.msys2.org/mingw/sources/" + filename,
            "scope": "Complete MSYS2 source-only package: upstream sources, PKGBUILD and downstream patches",
        }
        for component, version, filename in (
            ("msys2-gcc", "16.1.0-5", "mingw-w64-gcc-16.1.0-5.src.tar.zst"),
            ("msys2-libiconv", "1.19-1", "mingw-w64-libiconv-1.19-1.src.tar.zst"),
            (
                "msys2-winpthreads",
                "14.0.0.r92.g818fa6510-1",
                "mingw-w64-winpthreads-14.0.0.r92.g818fa6510-1.src.tar.zst",
            ),
        )
    ],
]


def observed_archive(item: dict) -> dict:
    """Record archive bytes at immutable reviewed commits during explicit locking."""
    if "sha256" in item:
        download_verified(item, SOURCE_CACHE)
        return item
    if LOCK.exists():
        previous = next(
            (
                entry
                for entry in json.loads(LOCK.read_text(encoding="utf-8"))["sources"]
                if entry["url"] == item["url"]
                and entry.get("commit") == item.get("commit")
            ),
            None,
        )
        if previous:
            download_verified(previous, SOURCE_CACHE)
            return previous
    SOURCE_CACHE.mkdir(parents=True, exist_ok=True)
    destination = SOURCE_CACHE / item["filename"]
    with tempfile.TemporaryDirectory(
        prefix="source-lock-", dir=SOURCE_CACHE
    ) as temporary:
        archive = Path(temporary) / item["filename"]
        request = urllib.request.Request(
            item["url"], headers={"User-Agent": "ManimStudio-Sources/2.2"}
        )
        with (
            urllib.request.urlopen(request, timeout=60) as response,
            archive.open("wb") as output,
        ):
            shutil.copyfileobj(response, output)
        record = {
            **item,
            "sha256": digest(archive),
            "hash_origin": "Archive observed at the reviewed upstream commit or exact distributor source-package version",
        }
        # Replace only the explicitly named cache archive after recording its hash.
        shutil.copy2(archive, destination)
    return record


def read_recipe(archive: Path, suffix: str) -> bytes:
    with tarfile.open(archive, "r:gz") as source:
        candidates = [
            member
            for member in source.getmembers()
            if member.name.endswith("/" + suffix) and member.isfile()
        ]
        if len(candidates) != 1:
            raise RuntimeError("Expected one upstream recipe: " + suffix)
        return source.extractfile(candidates[0]).read()


def package_sources(recipe: bytes) -> list[dict]:
    """Read only literal package identifiers, URLs and hashes from the pinned AST."""
    records = []
    for node in ast.walk(ast.parse(recipe)):
        if (
            not isinstance(node, ast.Call)
            or not isinstance(node.func, ast.Name)
            or node.func.id != "Package"
        ):
            continue
        values = {
            keyword.arg: ast.literal_eval(keyword.value)
            for keyword in node.keywords
            if keyword.arg in {"name", "source_url", "sha256", "source_filename"}
        }
        if not all(key in values for key in ("name", "source_url", "sha256")):
            raise RuntimeError(
                "A pinned source package has non-literal or missing provenance"
            )
        original = (
            values.get("source_filename") or values["source_url"].rsplit("/", 1)[-1]
        )
        records.append(
            {
                "component": values["name"],
                "filename": values["name"] + "-" + original,
                "url": values["source_url"],
                "sha256": values["sha256"],
                "hash_origin": "pyav-ffmpeg pinned scripts/pkg.py",
                "scope": "Includes a superset of Windows sources, including optional Linux components",
            }
        )
    if not records or len({item["component"] for item in records}) != len(records):
        raise RuntimeError("Invalid or duplicate upstream source catalog")
    return sorted(records, key=lambda item: item["component"])


def meson_sources(archive: Path, component: str) -> list[dict]:
    """Collect literal upstream hashes and build patches from frozen Meson wraps."""
    records = []
    with tarfile.open(archive, "r:gz") as source:
        for member in source.getmembers():
            if not member.isfile() or not member.name.endswith(".wrap"):
                continue
            parser = configparser.ConfigParser(interpolation=None)
            parser.read_string(source.extractfile(member).read().decode("utf-8"))
            if not parser.has_section("wrap-file"):
                # GVDB's exact revision is an explicit anchor. Gperf is an
                # unshipped build tool; its original branch reference remains
                # visible in the included recipe, rather than inventing a pin.
                continue
            wrap = parser["wrap-file"]
            for kind in ("source", "patch"):
                if kind + "_url" not in wrap:
                    continue
                url = (
                    wrap.get(kind + "_fallback_url", wrap[kind + "_url"])
                    if kind == "patch"
                    else wrap[kind + "_url"]
                )
                records.append(
                    {
                        "component": component
                        + "-"
                        + Path(member.name).stem
                        + "-"
                        + kind,
                        "filename": component + "-" + wrap[kind + "_filename"],
                        "url": url,
                        "sha256": wrap[kind + "_hash"],
                        "fallback_url": wrap.get(kind + "_fallback_url"),
                        "hash_origin": component
                        + " pinned "
                        + member.name.split("/", 1)[1],
                        "scope": "Static native libraries and their original Meson patches/build dependencies",
                    }
                )
    return records


def lock_sources() -> None:
    anchors = []
    for item in ANCHORS:
        print("Locking", item["component"], flush=True)
        anchors.append(observed_archive(item))
    pyav_recipe = SOURCE_CACHE / anchors[0]["filename"]
    packages = package_sources(read_recipe(pyav_recipe, "scripts/pkg.py"))
    for component in ("pango-build", "cairo-win-build"):
        anchor = next(item for item in anchors if item["component"] == component)
        packages.extend(meson_sources(SOURCE_CACHE / anchor["filename"], component))
    # Resolve only the sdist; its hash must already be present in the project lock.
    import tomllib

    runtime_lock = tomllib.loads((ROOT / "uv.lock").read_text(encoding="utf-8"))
    for name, component in (
        ("av", "pyav"),
        ("manimpango", "manimpango"),
        ("pycairo", "pycairo"),
    ):
        package = next(
            package for package in runtime_lock["package"] if package["name"] == name
        )
        source = package["sdist"]
        packages.append(
            {
                "component": component,
                "version": package["version"],
                "filename": component + "-" + package["version"] + ".tar.gz",
                "url": source["url"],
                "sha256": source["hash"].removeprefix("sha256:"),
                "hash_origin": "uv.lock",
            }
        )
    catalog = {
        "schema": 1,
        "release_version": VERSION,
        "recipe_origin": {"commit": anchors[0]["commit"], "path": "scripts/pkg.py"},
        "sources": anchors + packages,
    }
    LOCK.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")
    print("Wrote reviewed pinned source catalog:", LOCK.name, flush=True)


def snapshot_math() -> None:
    directory = CACHE / "math-runtime"
    snapshot = {
        "schema": 1,
        "product_version": "26.5",
        "origin": "Prepared private MiKTeX basic package repository snapshot",
        "warning": "This is a reviewed local snapshot, not an independently reproducible pinned upstream package repository. Online MiKTeX downloads are mutable.",
        "ghostscript_version": "9.25",
        "inventory": tree_inventory(directory),
    }
    (ROOT / "packaging" / "math-snapshot.json").write_text(
        json.dumps(snapshot, indent=2) + "\n", encoding="utf-8"
    )
    print("Recorded MiKTeX snapshot:", snapshot["inventory"]["tree_sha256"], flush=True)


def collect() -> None:
    catalog = json.loads(LOCK.read_text(encoding="utf-8"))
    records = []
    for item in catalog["sources"]:
        print("Verifying source:", item["component"], flush=True)
        source = download_verified(item, SOURCE_CACHE)
        records.append(
            {
                **item,
                "bytes": source.stat().st_size,
                "archive_path": "sources/" + source.name,
            }
        )
    math_recipe = (
        CACHE
        / "math-runtime"
        / "texmfs"
        / "install"
        / "source"
        / "miktex-ghostscript-bin-x64"
        / "compile_for_miktex.cmd"
    )
    manifest = {
        "schema": 1,
        "release_version": VERSION,
        "sources": records,
        "recipes": [
            {
                "path": "recipes/compile_for_miktex.cmd",
                "sha256": digest(math_recipe),
                "origin": "Bundled MiKTeX miktex-ghostscript-bin-x64 package source",
            }
        ],
        "coverage": {
            "included": [
                "PyAV sources and its pinned native codec sources/patches/build recipes",
                "Exact MSYS2 GCC 16.1.0-5, libiconv 1.19-1 and winpthreads 14.0.0.r92 source packages with original build scripts/patches",
                "ManimPango/Pycairo wrapper sources and frozen vendor recipes with all pinned static native sources and Meson patches",
                "MiKTeX 26.5 core source/build recipes",
                "Ghostscript 9.25 source and MiKTeX build recipe",
            ],
            "unresolved": [
                "Standalone FFmpeg tools until their separately verified source/recipe is added"
            ],
            "scope": "Source coverage for the identified GPL/LGPL/AGPL/MPL native runtime components. Permissive native wheels retain their original component notices. Unshipped build tools (MSVC, Meson, gperf) are specified in original upstream recipes.",
            "notice": "This artifact supplies the identified component sources and recipes directly; it is not a written source offer.",
        },
    }
    audit = ROOT / "packaging" / "native-source-audit.json"
    manifest["native_wheel_audit"] = json.loads(audit.read_text(encoding="utf-8"))
    extra = [
        (audit, "native-source-audit.json"),
        (ROOT / "packaging" / "tex-native-lock.json", "recipes/tex-native-lock.json"),
    ]
    embedded = []
    ffmpeg_manifest = CACHE / "ffmpeg-studio.manifest.json"
    if ffmpeg_manifest.exists():
        manifest["standalone_ffmpeg"] = json.loads(
            ffmpeg_manifest.read_text(encoding="utf-8")
        )
        from provenance import verify_file

        ffmpeg = manifest["standalone_ffmpeg"]
        native_archive = verify_file(CACHE / ffmpeg["archive"], ffmpeg["sha256"])
        with ZipFile(native_archive) as archive:
            built_recipe = archive.read("licenses/FFmpeg/build_ffmpeg.py")
        if hashlib.sha256(built_recipe).hexdigest() != ffmpeg["recipe_sha256"]:
            raise RuntimeError(
                "Embedded FFmpeg build recipe failed provenance verification"
            )
        embedded.append(("recipes/build_ffmpeg.py", built_recipe))
        manifest["recipes"].append(
            {
                "path": "recipes/build_ffmpeg.py",
                "sha256": ffmpeg["recipe_sha256"],
                "origin": "Exact authoritative recipe embedded in the verified native tools archive",
            }
        )
        for file in (ffmpeg_manifest,):
            if file.is_file():
                extra.append((file, "recipes/" + file.name))
                manifest["recipes"].append(
                    {"path": "recipes/" + file.name, "sha256": digest(file)}
                )
        current_recipe = ROOT / "packaging" / "build_ffmpeg.py"
        if digest(current_recipe) != ffmpeg["recipe_sha256"]:
            extra.append((current_recipe, "recipes/current-repository-build_ffmpeg.py"))
            manifest["recipes"].append(
                {
                    "path": "recipes/current-repository-build_ffmpeg.py",
                    "sha256": digest(current_recipe),
                    "origin": "Subsequently formatted repository recipe; see build_ffmpeg.py for the exact release build input",
                }
            )
        # The FFmpeg 9.0.2 archive is already included through PyAV's pinned catalog.
        manifest["coverage"]["unresolved"] = [
            item
            for item in manifest["coverage"]["unresolved"]
            if not item.startswith("Standalone FFmpeg")
        ]
        manifest["coverage"]["included"].append(
            "Standalone FFmpeg 9.0.2 source and separate build recipe/provenance"
        )
    encoded = json.dumps(manifest, indent=2) + "\n"
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    output = dist / f"Manim-Studio-{VERSION}-Third-Party-Sources.zip"
    handle, temporary = tempfile.mkstemp(prefix="sources-", suffix=".zip", dir=dist)
    import os

    os.close(handle)
    try:
        with ZipFile(temporary, "w", ZIP_STORED) as archive:
            for item in records:
                archive.write(SOURCE_CACHE / item["filename"], item["archive_path"])
            archive.write(math_recipe, "recipes/compile_for_miktex.cmd")
            archive.write(LOCK, "source-lock.json")
            archive.writestr("SOURCE-MANIFEST.json", encoded)
            archive.writestr(
                "READ-ME.txt",
                "Component sources and original build recipes. Consult SOURCE-MANIFEST.json for precise coverage and remaining gaps.\n",
            )
            for path, name in extra:
                archive.write(path, name)
            for name, contents in embedded:
                archive.writestr(name, contents)
        with ZipFile(temporary) as archive:
            if archive.testzip() is not None:
                raise RuntimeError("Source archive failed CRC verification")
        os.replace(temporary, output)
    finally:
        Path(temporary).unlink(missing_ok=True)
    (dist / "SOURCE-MANIFEST.json").write_text(encoded, encoding="utf-8")
    write_checksums(dist, VERSION)
    print("Created source asset:", output.name, flush=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--lock",
        action="store_true",
        help="Explicitly capture archives at the reviewed immutable upstream commits",
    )
    parser.add_argument(
        "--snapshot-math",
        action="store_true",
        help="Explicitly accept the reviewed local MiKTeX tree",
    )
    args = parser.parse_args()
    if args.lock:
        lock_sources()
    if args.snapshot_math:
        snapshot_math()
    if not args.lock and not args.snapshot_math:
        collect()


if __name__ == "__main__":
    main()
