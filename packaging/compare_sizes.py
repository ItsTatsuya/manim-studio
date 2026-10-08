"""Measure pristine portable ZIP payloads, including component percentages."""

import argparse
from collections import defaultdict
import json
from pathlib import Path
from zipfile import ZipFile


def component(parts):
    if parts[0] == "webview2" or parts[-1] == "MicrosoftEdgeWebview2Setup.exe":
        return "WebView2"
    if parts[0] == "math":
        return "TeX"
    if parts[0] == "runtime":
        if len(parts) > 3 and parts[1:3] == ("Lib", "site-packages"):
            if parts[3] == "typst" or parts[3].startswith("typst-"):
                return "Typst"
        return "Python + Manim"
    return "App, tools + licenses"


def measure(path):
    sizes = defaultdict(int)
    compressed = defaultdict(int)
    with ZipFile(path) as archive:
        invalid = archive.testzip()
        if invalid:
            raise RuntimeError("ZIP CRC failed: " + invalid)
        count = 0
        for entry in archive.infolist():
            if entry.is_dir():
                continue
            parts = tuple(Path(entry.filename).parts[1:])
            if not parts:
                raise RuntimeError("Portable ZIP must have one root folder")
            category = component(parts)
            sizes[category] += entry.file_size
            compressed[category] += entry.compress_size
            count += 1
    total = sum(sizes.values())
    return {
        "archive": str(path.resolve()),
        "zip_bytes": path.stat().st_size,
        "payload_bytes": total,
        "file_count": count,
        "components": {
            name: {
                "bytes": size,
                "percent": round(size / total * 100, 4),
                "compressed_bytes": compressed[name],
            }
            for name, size in sizes.items()
        },
        "zip_overhead_bytes": path.stat().st_size - sum(compressed.values()),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, nargs="+", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    before = measure(args.before)
    candidates = [measure(path) for path in args.after]
    for candidate in candidates:
        candidate["payload_reduction_percent"] = round(
            (1 - candidate["payload_bytes"] / before["payload_bytes"]) * 100, 4
        )
        candidate["zip_reduction_percent"] = round(
            (1 - candidate["zip_bytes"] / before["zip_bytes"]) * 100, 4
        )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps({"before": before, "after": candidates}, indent=2) + "\n",
        encoding="utf-8",
    )
    print(args.output)


if __name__ == "__main__":
    main()
