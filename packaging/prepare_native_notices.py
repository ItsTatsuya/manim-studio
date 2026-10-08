"""Install original native component notices from hash-verified source archives."""

from __future__ import annotations

import json
from pathlib import Path
import shutil
import subprocess
import tarfile

from provenance import CACHE, ROOT, download_verified, digest


def install_native_notices(bundle: Path) -> dict:
    destination = bundle / "licenses" / "Native-Libraries"
    destination.mkdir(parents=True, exist_ok=True)
    catalog = json.loads(
        (ROOT / "packaging" / "source-lock.json").read_text(encoding="utf-8")
    )
    records = []
    for item in catalog["sources"]:
        if (
            item["filename"].endswith((".zip", ".zst"))
            or item["component"] == "w64devkit-source"
        ):
            continue
        source = download_verified(item, CACHE / "third-party-sources")
        with tarfile.open(source, "r:*") as archive:
            for member in archive.getmembers():
                name = Path(member.name).name
                # Preserve original component licenses and attribution, including
                # vendored MiKTeX libraries, without unpacking upstream code.
                if not member.isfile() or not name.upper().startswith(
                    ("LICENSE", "COPYING", "COPYRIGHT", "NOTICE")
                ):
                    continue
                if member.size > 2_000_000:
                    raise RuntimeError("Unexpectedly large native license file")
                relative = Path(*Path(member.name).parts[1:])
                if relative.is_absolute() or ".." in relative.parts:
                    raise RuntimeError("Unsafe native license path")
                output = destination / item["component"] / relative
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_bytes(archive.extractfile(member).read())
                records.append(
                    {
                        "path": output.relative_to(bundle).as_posix(),
                        "sha256": digest(output),
                        "source_component": item["component"],
                        "source_sha256": item["sha256"],
                    }
                )
    audit_file = ROOT / "packaging" / "native-source-audit.json"
    audit = json.loads(audit_file.read_text(encoding="utf-8"))
    for item in audit["binary_distributor_packages"]:
        archive = download_verified(item, CACHE / "native-binary-provenance")
        paths = subprocess.run(
            ["tar", "-tf", str(archive)],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        ).stdout.splitlines()
        for path in paths:
            if path.endswith("/") or not (
                "/share/licenses/" in path or path in {".BUILDINFO", ".PKGINFO"}
            ):
                continue
            relative = Path(path)
            if relative.is_absolute() or ".." in relative.parts:
                raise RuntimeError("Unsafe distributor license path")
            output = destination / item["component"] / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            contents = subprocess.run(
                ["tar", "-xOf", str(archive), path], check=True, capture_output=True
            ).stdout
            output.write_bytes(contents)
            records.append(
                {
                    "path": output.relative_to(bundle).as_posix(),
                    "sha256": digest(output),
                    "source_component": item["component"],
                    "distributor_package_sha256": item["sha256"],
                }
            )
    shutil.copy2(audit_file, destination / audit_file.name)
    (destination / "NOTICE-MANIFEST.json").write_text(
        json.dumps({"schema": 1, "files": records}, indent=2) + "\n", encoding="utf-8"
    )
    return {
        "manifest": "licenses/Native-Libraries/NOTICE-MANIFEST.json",
        "files": len(records),
        "audit_sha256": digest(audit_file),
        "source_lock_sha256": digest(ROOT / "packaging" / "source-lock.json"),
    }


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    print(install_native_notices(args.bundle))
