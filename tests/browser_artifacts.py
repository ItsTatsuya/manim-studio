"""Keep diagnostic screenshots outside application data and tracked sources."""

import os
from pathlib import Path
import tempfile


def artifact_directory():
    configured = os.environ.get("MANIM_STUDIO_TEST_ARTIFACTS")
    directory = (
        Path(configured)
        if configured
        else Path(tempfile.mkdtemp(prefix="manim-studio-browser-"))
    )
    directory.mkdir(parents=True, exist_ok=True)
    print(f"Browser screenshots: {directory}", flush=True)
    return directory
