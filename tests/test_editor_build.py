"""The generated HTML must invalidate cached assets after an app update."""

import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(
    shutil.which("node") and (ROOT / "node_modules/esbuild").is_dir(),
    "Run npm ci and install Node.js to verify the editor build",
)
class EditorBuildTests(unittest.TestCase):
    def test_changed_assets_get_new_cache_keys_and_build_is_repeatable(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            ui = project / "studio_ui"
            ui.mkdir()
            (ui / "editor.js").write_text("window.testEditor = true;", encoding="utf-8")
            assets = (
                "style.css",
                "app.js",
                "workspace.js",
                "updates.js",
                "vendor/editor.bundle.js",
            )
            for asset in assets[:-1]:
                (ui / asset).write_text("/* first version */", encoding="utf-8")
            (ui / "index.html").write_text(
                "\n".join(
                    f'<script src="/static/{asset}?v=old"></script>' for asset in assets
                ),
                encoding="utf-8",
            )

            def build():
                subprocess.run(
                    [shutil.which("node"), str(ROOT / "packaging/build_editor.mjs")],
                    cwd=project,
                    check=True,
                    capture_output=True,
                    text=True,
                )
                html = (ui / "index.html").read_text(encoding="utf-8")
                for asset in assets:
                    digest = hashlib.sha256((ui / asset).read_bytes()).hexdigest()[:12]
                    self.assertIn(f'/static/{asset}?v={digest}"', html)
                self.assertNotIn("?v=old", html)
                return html

            first = build()
            (ui / "app.js").write_text("/* changed behavior */", encoding="utf-8")
            second = build()
            self.assertNotEqual(first, second)
            self.assertEqual(second, build())


if __name__ == "__main__":
    unittest.main()
