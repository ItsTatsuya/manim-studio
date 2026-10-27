"""Isolated browser fixture: real Studio server with deterministic CLI processes."""

import argparse
import os
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import agents
import file_picker
import narration
import studio
import uvicorn

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, required=True)
parser.add_argument("--native-file-picker", action="store_true")
args = parser.parse_args()
studio.save_preferences(studio.Preferences(values={"studio-onboarding": "2"}))
file_picker.AVAILABLE = args.native_file_picker and os.name == "nt"
if file_picker.AVAILABLE:
    native_choose = file_picker.choose
    file_picker.choose = lambda kind: native_choose(kind, owner=0)

agents.resolve_binary = lambda provider, configured="": "fixture-cli"
# Agent CLI updates never reach the network or change an install here.
studio.AGENT_UPDATES.automatic = False
studio.AGENT_UPDATES.fetch_latest = lambda provider: "1.2.3"
agents.launch_command = lambda binary: [
    sys.executable,
    str(ROOT / "tests/fake_agent_cli.py"),
]
models = os.environ.get("MANIM_STUDIO_TEST_KOKORO")
if models:
    target = narration.model_dir(studio.ROOT)
    target.mkdir(parents=True, exist_ok=True)
    for item in (narration.FILES["standard"], narration.FILES["voices"]):
        shutil.copy2(Path(models) / item["name"], target / item["name"])

uvicorn.run(studio.app, host="127.0.0.1", port=args.port, log_level="warning")
