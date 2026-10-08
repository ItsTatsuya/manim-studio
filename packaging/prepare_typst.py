"""Preserve Typst's wheel SBOM and original notices for its pinned Rust crates."""

from __future__ import annotations

import json
from pathlib import Path
import tarfile
import tomllib
from zipfile import ZipFile

from provenance import ROOT, CACHE, digest, download_verified


def typst_sources() -> tuple[dict, list[dict]]:
    lock = tomllib.loads((ROOT / "uv.lock").read_text(encoding="utf-8"))
    package = next(package for package in lock["package"] if package["name"] == "typst")
    wheel = next(
        wheel
        for wheel in package["wheels"]
        if wheel["url"].endswith("cp38-abi3-win_amd64.whl")
    )
    item = {
        "filename": wheel["url"].rsplit("/", 1)[-1],
        "url": wheel["url"],
        "sha256": wheel["hash"].removeprefix("sha256:"),
    }
    path = download_verified(item, CACHE / "typst-provenance")
    with ZipFile(path) as archive:
        names = [
            name
            for name in archive.namelist()
            if "/sboms/" in name and name.endswith(".json")
        ]
        if len(names) != 1:
            raise RuntimeError(
                "Pinned Typst wheel must provide its Rust dependency SBOM"
            )
        sbom = json.loads(archive.read(names[0]))
    sdist = package["sdist"]
    records = [
        {
            "component": "typst-python",
            "version": package["version"],
            "filename": "typst-" + package["version"] + ".tar.gz",
            "url": sdist["url"],
            "sha256": sdist["hash"].removeprefix("sha256:"),
            "hash_origin": "uv.lock",
        }
    ]
    for component in sbom["components"]:
        name, version = component["name"], component["version"]
        hashes = [
            record["content"]
            for record in component.get("hashes", [])
            if record["alg"] == "SHA-256"
        ]
        if len(hashes) != 1 or not component.get("purl", "").startswith("pkg:cargo/"):
            raise RuntimeError("Typst SBOM has an unpinned native component: " + name)
        records.append(
            {
                "component": "typst-crate-" + name,
                "version": version,
                "filename": name + "-" + version + ".crate",
                "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
                "sha256": hashes[0],
                "hash_origin": "SHA256 in hash-locked Typst Windows wheel CycloneDX SBOM",
                "licenses": component.get("licenses", []),
            }
        )
    return {"wheel_sha256": item["sha256"], "sbom": sbom}, records


def install_typst_notices(bundle: Path) -> dict:
    provenance, records = typst_sources()
    destination = bundle / "licenses/Typst"
    destination.mkdir(parents=True, exist_ok=True)
    manifest = []
    for record in records:
        archive = download_verified(record, CACHE / "third-party-sources")
        with tarfile.open(archive, "r:gz") as source:
            for member in source.getmembers():
                if not member.isfile() or not Path(member.name).name.upper().startswith(
                    ("LICENSE", "COPYING", "COPYRIGHT", "NOTICE")
                ):
                    continue
                relative = Path(*Path(member.name).parts[1:])
                if (
                    relative.is_absolute()
                    or ".." in relative.parts
                    or member.size > 2_000_000
                ):
                    raise RuntimeError("Unsafe Typst notice path")
                output = destination / record["component"] / relative
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_bytes(source.extractfile(member).read())
                manifest.append(
                    {
                        "path": output.relative_to(bundle).as_posix(),
                        "sha256": digest(output),
                        "source_sha256": record["sha256"],
                    }
                )
    (destination / "SOURCE-MANIFEST.json").write_text(
        json.dumps(
            {
                "wheel_sha256": provenance["wheel_sha256"],
                "sources": records,
                "notices": manifest,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    return {
        "manifest": "licenses/Typst/SOURCE-MANIFEST.json",
        "wheel_sha256": provenance["wheel_sha256"],
        "source_packages": len(records),
        "notices": len(manifest),
    }


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    print(install_typst_notices(args.bundle))
