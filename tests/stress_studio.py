"""Stress and edge-case run against a real Studio server (not part of discovery).

Usage: uv run python tests/stress_studio.py [--kokoro FOLDER]

Starts an isolated server, drives near-limit requests through the public API and
prints one PASS/FAIL line per scenario. Needs the Kokoro model files for the
narration scenarios; everything else runs without them.
"""

import argparse
import json
import re
import shutil
import socket
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import uvicorn  # noqa: E402

import narration  # noqa: E402
import studio  # noqa: E402

SCENE = (
    "from manim import *\nclass MainScene(Scene):\n    def construct(self):\n"
    "        self.add(Circle())\n        self.wait(0.2)\n"
)
RESULTS = []


def server(kokoro):
    temporary = tempfile.TemporaryDirectory()
    studio.JOBS, studio.ACTIVE, studio.PROCESS, studio.WORKER = {}, None, None, None
    studio.ROOT = Path(temporary.name)
    studio.OUTPUTS = studio.ROOT / "outputs"
    studio.OUTPUTS.mkdir()
    studio.RENDERS = studio.ROOT / "renders"
    if kokoro:
        target = narration.model_dir(studio.ROOT)
        target.mkdir(parents=True)
        for item in (narration.FILES["voices"], narration.FILES["standard"]):
            shutil.copy2(Path(kokoro) / item["name"], target / item["name"])
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    url = f"http://127.0.0.1:{sock.getsockname()[1]}"
    instance = uvicorn.Server(uvicorn.Config(studio.app, log_level="error"))
    threading.Thread(target=lambda: instance.run(sockets=[sock]), daemon=True).start()
    while not instance.started:
        time.sleep(0.05)
    page = urllib.request.urlopen(url).read().decode()
    token = re.search(r'name="studio-token" content="([^"]+)"', page)[1]
    return temporary, instance, url, token


def call(url, token, path, body=None, method=None):
    request = urllib.request.Request(
        url + path,
        data=None if body is None else json.dumps(body).encode(),
        method=method or ("GET" if body is None else "POST"),
        headers={"Content-Type": "application/json", "X-Studio-Token": token},
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return response.status, json.loads(response.read() or b"null")
    except urllib.error.HTTPError as error:
        try:
            return error.code, json.loads(error.read())
        except ValueError:
            return error.code, None


def wait(url, token, job_id, limit=1800):
    deadline = time.monotonic() + limit
    while time.monotonic() < deadline:
        _, current = call(url, token, "/api/renders/" + job_id)
        if current["status"] in {"completed", "failed", "cancelled"}:
            while studio.ACTIVE and time.monotonic() < deadline:
                time.sleep(0.05)
            return current
        time.sleep(0.25)
    raise TimeoutError("render did not finish")


def check(name, ok, detail=""):
    RESULTS.append(ok)
    print(("PASS " if ok else "FAIL ") + name + (f" - {detail}" if detail else ""))


def render(url, token, body, limit=1800):
    status, job = call(url, token, "/api/render", body)
    if status != 202:
        return status, job
    return status, wait(url, token, job["id"], limit)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--kokoro", help="Folder with the Kokoro model files")
    parser.add_argument("--long", action="store_true", help="Include slow scenarios")
    args = parser.parse_args()
    temporary, instance, url, token = server(args.kokoro)
    try:
        small = {"width": 128, "height": 72, "fps": 10}

        # ---- request limits -------------------------------------------------
        status, _ = call(
            url, token, "/api/render", {"source": "", "scene": "MainScene"}
        )
        check("empty source rejected", status == 422, status)
        status, _ = call(
            url,
            token,
            "/api/render",
            {"source": SCENE + "#" * 500_001, "scene": "MainScene"},
        )
        check("source over 500k chars rejected", status == 422, status)
        status, result = render(
            url,
            token,
            {
                "source": SCENE + "#" * (499_000 - len(SCENE)),
                "scene": "MainScene",
                **small,
            },
            120,
        )
        check(
            "source near 500k chars renders",
            status == 202 and result["status"] == "completed",
            (result or {}).get("error") if isinstance(result, dict) else status,
        )
        status, result = render(
            url,
            token,
            {"source": SCENE, "scene": "MainScene", "name": "É" * 100, **small},
            120,
        )
        check(
            "100-char unicode name",
            status == 202 and result["status"] == "completed",
            result,
        )
        status, _ = call(
            url,
            token,
            "/api/render",
            {"source": SCENE, "scene": "MainScene", "name": "x" * 101},
        )
        check("101-char name rejected", status == 422, status)
        for label, extra, expect in (
            ("odd width", {"width": 127, "height": 72}, 422),
            ("zero fps", {"width": 128, "height": 72, "fps": 0}, 422),
            ("negative fps", {"width": 128, "height": 72, "fps": -5}, 422),
            ("huge resolution", {"width": 99999, "height": 99998, "fps": 10}, 422),
            ("width only", {"width": 128}, 422),
        ):
            status, _ = call(
                url,
                token,
                "/api/render",
                {"source": SCENE, "scene": "MainScene", **extra},
            )
            check(f"{label} rejected", status == expect, status)
        status, result = render(
            url,
            token,
            {"source": SCENE, "scene": "MainScene", "width": 2, "height": 2, "fps": 1},
            120,
        )
        check(
            "2x2 at 1 fps renders or fails cleanly",
            status == 202 and result["status"] in {"completed", "failed"},
            (result or {}).get("error") if isinstance(result, dict) else status,
        )

        # ---- narration request limits ---------------------------------------
        base = {"source": SCENE, "scene": "MainScene", **small}
        status, _ = call(
            url,
            token,
            "/api/render",
            {**base, "narration": {"enabled": True, "text": "x" * 100_001}},
        )
        check("narration over 100k chars rejected", status == 422, status)
        status, body = call(
            url,
            token,
            "/api/render",
            {
                **base,
                "narration": {"enabled": True, "text": "\n\n".join(["word"] * 401)},
            },
        )
        check("401 narration lines rejected", status == 422, body)
        status, body = call(
            url,
            token,
            "/api/render",
            {**base, "narration": {"enabled": True, "text": "[99999] hello"}},
        )
        check("timestamp past one hour rejected", status == 422, body)
        status, _ = call(
            url,
            token,
            "/api/render",
            {**base, "narration": {"enabled": True, "text": "hi", "speed": 9}},
        )
        check("speed out of range rejected", status == 422, status)
        status, _ = call(
            url,
            token,
            "/api/render",
            {**base, "narration": {"enabled": True, "text": "hi", "voice": "zz_nope"}},
        )
        check("unknown voice rejected", status == 422, status)

        if not args.kokoro:
            print("Skipping narrated renders: pass --kokoro FOLDER")
        else:
            paragraph = (
                "The derivative of a function measures how fast its output changes "
                "as its input changes, and this simple idea unlocks most of calculus. "
            )
            cases = {
                "1500-char paragraph": paragraph * 10,
                "no punctuation, 1200 chars": "word " * 240,
                "emoji and symbols only": "🙂 ... ≈ → ∞",
                "non-English text": "你好，世界。这是一个测试。",
                "very long URL": "See https://example.com/" + "a" * 900 + " for more.",
                "mixed math notation": "If x^2 + y^2 = r^2 and 3/4 of π ≈ 2.36, then f(x) → ∞.",
            }
            for label, text in cases.items():
                status, result = render(
                    url,
                    token,
                    {**base, "narration": {"enabled": True, "text": text}},
                    900,
                )
                detail = ""
                if status == 202:
                    detail = result.get("error") or "; ".join(
                        result.get("warnings", [])
                    )
                check(
                    f"narration: {label}",
                    status == 202 and result["status"] == "completed",
                    detail[:200],
                )

            # Checkpoint: identical animation, edited narration reuses the video.
            script = SCENE.replace("self.wait(0.2)", "self.wait(1)")
            first = {
                **base,
                "source": script,
                "narration": {"enabled": True, "text": "First pass."},
            }
            started = time.monotonic()
            _, one = render(url, token, first, 600)
            first_time = time.monotonic() - started
            second = {
                **first,
                "narration": {
                    "enabled": True,
                    "text": "First pass.\n\nSecond line added.",
                },
            }
            started = time.monotonic()
            _, two = render(url, token, second, 600)
            second_time = time.monotonic() - started
            log = (studio.output_directory(two) / "render.log").read_text(
                encoding="utf-8"
            )
            check(
                "edited narration reuses the rendered animation",
                two["status"] == "completed" and "Reusing the animation" in log,
                f"{first_time:.1f}s then {second_time:.1f}s",
            )

            # Many lines near the practical limit.
            lines = "\n\n".join(
                f"Line number {n} of the long narration." for n in range(60)
            )
            status, result = render(
                url,
                token,
                {**base, "narration": {"enabled": True, "text": lines}},
                1500,
            )
            check(
                "60-line narration",
                status == 202 and result["status"] == "completed",
                (result or {}).get("error") if isinstance(result, dict) else status,
            )
            if args.long:
                text = "\n\n".join(
                    "[%d:%02d] %s" % (n * 8 // 60, n * 8 % 60, paragraph)
                    for n in range(100)
                )
                status, result = render(
                    url,
                    token,
                    {**base, "narration": {"enabled": True, "text": text}},
                    3600,
                )
                check(
                    "100 timed lines of long text",
                    status == 202 and result["status"] == "completed",
                    (result or {}).get("error") if isinstance(result, dict) else status,
                )
    finally:
        studio.shutdown_render()
        instance.should_exit = True
        temporary.cleanup()
    print(f"\n{sum(RESULTS)}/{len(RESULTS)} scenarios passed")
    raise SystemExit(0 if all(RESULTS) else 1)


if __name__ == "__main__":
    main()
