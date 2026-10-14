"""Pin signed Microsoft Evergreen installers; normal builds only reuse these pins."""

from __future__ import annotations

import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import shutil
import urllib.request
from urllib.parse import urlparse

from provenance import CACHE, ROOT, digest, download_verified

LOCK = ROOT / "packaging" / "webview-evergreen.json"
LINKS = {
    "bootstrapper": (
        "MicrosoftEdgeWebview2Setup.exe",
        "https://go.microsoft.com/fwlink/p/?LinkId=2124703",
    ),
    "offline": (
        "MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
        "https://go.microsoft.com/fwlink/?linkid=2124701",
    ),
}


def verify_signature(path: Path) -> dict:
    environment = dict(os.environ, MANIM_SIGNATURE_PATH=str(path))
    # PowerShell 7's module path must not override Windows PowerShell's modules.
    environment.pop("PSModulePath", None)
    script = (
        "$signature = Get-AuthenticodeSignature -LiteralPath $env:MANIM_SIGNATURE_PATH; "
        "@{status=$signature.Status.ToString(); "
        "subject=$signature.SignerCertificate.Subject; "
        "thumbprint=$signature.SignerCertificate.Thumbprint} | ConvertTo-Json -Compress"
    )
    result = subprocess.run(
        [
            shutil.which("pwsh.exe") or "powershell.exe",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            base64.b64encode(script.encode("utf-16-le")).decode("ascii"),
        ],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        raise RuntimeError(result.stderr)
    signature = json.loads(result.stdout)
    if signature["status"] != "Valid" or "O=Microsoft Corporation" not in signature.get(
        "subject", ""
    ):
        raise RuntimeError("Evergreen installer must have a valid Microsoft signature")
    return signature


def capture() -> None:
    folder = CACHE / "webview-evergreen"
    folder.mkdir(exist_ok=True)
    records = {}
    for kind, (name, link) in LINKS.items():
        print("Pinning Microsoft Evergreen", kind, flush=True)
        request = urllib.request.Request(
            link, headers={"User-Agent": "ManimStudio/1.0"}
        )
        with urllib.request.urlopen(request, timeout=60) as response:
            url = response.geturl()
            if not url.startswith("https://") or not (
                urlparse(url).hostname or ""
            ).endswith(".microsoft.com"):
                raise RuntimeError("Unexpected Microsoft installer download origin")
            path = folder / name
            with path.open("wb") as output:
                shutil.copyfileobj(response, output)
        signature = verify_signature(path)
        records[kind] = {
            "filename": name,
            "url": url,
            "sha256": digest(path),
            "bytes": path.stat().st_size,
            "signature": signature,
        }
    LOCK.write_text(
        json.dumps(
            {
                "schema": 1,
                "distribution": "Evergreen",
                "reference": "https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution",
                "installers": records,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def installers() -> tuple[dict, dict[str, Path]]:
    record = json.loads(LOCK.read_text(encoding="utf-8"))
    files = {
        kind: download_verified(item, CACHE / "webview-evergreen")
        for kind, item in record["installers"].items()
    }
    return record, files


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lock", action="store_true")
    args = parser.parse_args()
    if args.lock:
        capture()
    else:
        print(installers()[0])
