"""Run current workspace checks against MANIM_STUDIO_URL."""

from pathlib import Path
import runpy

root = Path(__file__).resolve().parent
for name in (
    "workspace_layout.py",
    "workspace_flow.py",
    "workspace_responsive.py",
    "workspace_accessibility.py",
    "workspace_regressions.py",
    "workspace_state_regressions.py",
    "workspace_output_settings.py",
    "workspace_naming.py",
    "workspace_updates.py",
):
    runpy.run_path(str(root / name), run_name="__main__")
