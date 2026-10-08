"""Run current native UI checks with an isolated MANIM_STUDIO_DATA folder."""

from pathlib import Path
import runpy
import sys

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root))
runpy.run_path(str(root / "packaging" / "verify_native.py"), run_name="__main__")
