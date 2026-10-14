"""Vite output must be repeatable and use new URLs when UI content changes."""

import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(
    shutil.which("node") and (ROOT / "node_modules/vite").is_dir(),
    "Run npm ci to verify the UI build",
)
class EditorBuildTests(unittest.TestCase):
    def test_changed_assets_get_new_cache_keys_and_build_is_repeatable(self):
        # Copy the small source tree next to node_modules so config imports resolve.
        with tempfile.TemporaryDirectory(
            prefix=".ui-build-test-", dir=ROOT
        ) as directory:
            project = Path(directory)
            shutil.copytree(ROOT / "web", project / "web")
            ui = project / "studio_ui"

            def build():
                result = subprocess.run(
                    [
                        shutil.which("node"),
                        str(ROOT / "node_modules/vite/bin/vite.js"),
                        "build",
                        "--config",
                        str(project / "web/vite.config.ts"),
                    ],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    encoding="utf-8",
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                html = (ui / "index.html").read_text(encoding="utf-8")
                urls = re.findall(r'(?:src|href)="(/static/assets/[^\"]+)"', html)
                # One entry script and stylesheet, plus preloads of shared chunks.
                self.assertEqual(len(re.findall(r"<script[^>]+src=", html)), 1)
                self.assertEqual(len(re.findall(r'rel="stylesheet"', html)), 1)
                self.assertGreaterEqual(len(urls), 2)
                for url in urls:
                    self.assertRegex(url, r"-[A-Za-z0-9_-]+\.(?:css|js)$")
                    self.assertTrue((ui / url.removeprefix("/static/")).is_file())
                self.assertIn(
                    "T3 Code",
                    (ui / "THIRD-PARTY-LICENSES.txt").read_text(encoding="utf-8"),
                )
                self.assertTrue((ui / "fonts/OFL.txt").is_file())
                return {
                    file.relative_to(ui).as_posix(): hashlib.sha256(
                        file.read_bytes()
                    ).hexdigest()
                    for file in ui.rglob("*")
                    if file.is_file()
                }

            first = build()
            source = project / "web/src/components/HomeRedirect.tsx"
            source.write_text(
                source.read_text(encoding="utf-8").replace(
                    "Make your first animation", "Create your first animation"
                ),
                encoding="utf-8",
            )
            second = build()
            self.assertNotEqual(first, second)
            self.assertEqual(second, build())


if __name__ == "__main__":
    unittest.main()
