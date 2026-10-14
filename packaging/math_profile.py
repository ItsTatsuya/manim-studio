"""Derive a minimal payload from the verified original MiKTeX snapshot."""

from __future__ import annotations

from collections import deque
import json
from pathlib import Path
import shutil

import pefile

from provenance import ROOT, digest, tree_inventory, verify_math_snapshot

BIN = Path("texmfs/install/miktex/bin/x64")


def native_closure(directory: Path, roots: list[str]) -> set[Path]:
    files = {path.name.lower(): path for path in directory.iterdir() if path.is_file()}
    pending = deque(roots)
    # Qt plugins and launcher helpers are loaded dynamically rather than through
    # executable import tables. Retain them and walk their imports too.
    pending.extend(
        str(path) for path in directory.rglob("*.dll") if path.parent != directory
    )
    seen = set()
    while pending:
        value = pending.popleft()
        path = files.get(value.lower()) if Path(value).name == value else Path(value)
        if path is None or not path.is_file():
            raise RuntimeError("Missing minimal TeX engine: " + value)
        if path in seen:
            continue
        seen.add(path)
        with pefile.PE(str(path), fast_load=True) as binary:
            binary.parse_data_directories(directories=[1, 13])
            imports = getattr(binary, "DIRECTORY_ENTRY_IMPORT", []) + getattr(
                binary, "DIRECTORY_ENTRY_DELAY_IMPORT", []
            )
            pending.extend(
                entry.dll.decode().lower()
                for entry in imports
                if entry.dll.decode().lower() in files
            )
    return seen


def minimal_paths(source: Path, policy: dict) -> set[Path]:
    binaries = native_closure(source / BIN, policy["roots"])
    selected = set()
    for path in source.rglob("*"):
        if not path.is_file() or "__pycache__" in path.parts or path.suffix == ".pyc":
            continue
        relative = path.relative_to(source)
        if relative.parent == BIN and path.suffix.lower() in {".exe", ".dll"}:
            if path not in binaries:
                continue
        # Preserve original licenses even when surrounding documentation is pruned.
        license_file = path.name.upper().startswith(
            ("LICENSE", "COPYING", "NOTICE", "COPYRIGHT")
        )
        if not license_file and any(
            relative.as_posix().startswith(prefix)
            for prefix in policy["excluded_prefixes"]
        ):
            continue
        selected.add(relative)
    return selected


def install_minimal_math(source: Path, destination: Path) -> dict:
    original = verify_math_snapshot(source)
    policy_path = ROOT / "packaging/minimal-tex.json"
    policy = json.loads(policy_path.read_text(encoding="utf-8"))
    paths = minimal_paths(source, policy)
    if destination.exists():
        raise RuntimeError("Minimal math destination must be a fresh directory")
    for relative in sorted(paths):
        target = destination / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source / relative, target)
    inventory = tree_inventory(destination)
    original_records = {
        entry["path"]: entry for entry in original["inventory"]["files"]
    }
    if any(
        original_records.get(entry["path"]) != entry for entry in inventory["files"]
    ):
        raise RuntimeError(
            "Minimal TeX payload does not match the reviewed source snapshot"
        )
    return {
        "profile": "minimal",
        "policy_sha256": digest(policy_path),
        "source_tree_sha256": original["inventory"]["tree_sha256"],
        "inventory": inventory,
        "limitations": policy["limitations"],
    }
