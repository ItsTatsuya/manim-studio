"""Runtime migration must fail before replacement when dependencies are missing."""

import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from fastapi import HTTPException
import studio
import webview_runtime

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "studio_bootstrap", ROOT / "packaging/bootstrap.py"
)
bootstrap = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bootstrap)


class WebViewRuntimeTests(unittest.TestCase):
    def test_shared_runtime_skips_installer_entirely(self):
        with (
            patch.object(
                webview_runtime, "evergreen_version", return_value="154.0.1.2"
            ),
            patch.object(webview_runtime.subprocess, "run") as run,
        ):
            self.assertEqual(
                webview_runtime.ensure_evergreen(Path("missing-package")), "154.0.1.2"
            )
            run.assert_not_called()

    def test_first_run_verifies_installer_and_rechecks_installation(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools").mkdir()
            (root / "app").mkdir()
            binary = root / "tools/bootstrap.exe"
            binary.write_bytes(b"reviewed Microsoft bootstrapper fixture")
            manifest = root / "app/webview-evergreen.json"
            record = {
                "filename": binary.name,
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            }
            manifest.write_text(json.dumps({"installers": {"bootstrapper": record}}))
            with (
                patch.object(
                    webview_runtime,
                    "evergreen_version",
                    side_effect=[None, "154.0.1.2"],
                ),
                patch.object(
                    webview_runtime.subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess([], 0),
                ) as run,
                patch.object(
                    webview_runtime.subprocess, "CREATE_NO_WINDOW", 0, create=True
                ),
            ):
                self.assertEqual(webview_runtime.ensure_evergreen(root), "154.0.1.2")
                self.assertEqual(
                    run.call_args.args[0], [str(binary), "/silent", "/install"]
                )
            binary.write_bytes(b"changed installer")
            with (
                patch.object(webview_runtime, "evergreen_version", return_value=None),
                patch.object(webview_runtime.subprocess, "run") as run,
            ):
                with self.assertRaisesRegex(RuntimeError, "verification failed"):
                    webview_runtime.ensure_evergreen(root)
                run.assert_not_called()

    def test_success_exit_without_a_runtime_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "tools").mkdir()
            (root / "app").mkdir()
            binary = root / "tools/bootstrap.exe"
            binary.write_bytes(b"fixture")
            (root / "app/webview-evergreen.json").write_text(
                json.dumps(
                    {
                        "installers": {
                            "bootstrapper": {
                                "filename": binary.name,
                                "sha256": hashlib.sha256(
                                    binary.read_bytes()
                                ).hexdigest(),
                            }
                        }
                    }
                )
            )
            with (
                patch.object(webview_runtime, "evergreen_version", return_value=None),
                patch.object(
                    webview_runtime.subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess([], 0),
                ),
                patch.object(
                    webview_runtime.subprocess, "CREATE_NO_WINDOW", 0, create=True
                ),
            ):
                with self.assertRaisesRegex(RuntimeError, "Runtime is required"):
                    webview_runtime.ensure_evergreen(root)

    def test_legacy_metadata_keeps_fixed_browser_mode(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "standalone.json").write_text('{"version":"1.0.0"}')
            self.assertEqual(webview_runtime.browser_mode(root), "fixed")
            (root / "standalone.json").write_text('{"webview2":"evergreen"}')
            self.assertEqual(webview_runtime.browser_mode(root), "evergreen")


class EquationRuntimeTests(unittest.TestCase):
    def test_typst_scripts_do_not_require_latex(self):
        source = "from manim import *\nclass Demo(Scene):\n    def construct(self):\n        self.add(MathTypst('x^2'))\n"
        result = studio.analyze(source)
        self.assertTrue(result["needs_typst"])
        self.assertFalse(result["needs_latex"])
        with patch.object(
            studio, "health", return_value={"latex": True, "typst": False}
        ):
            with self.assertRaises(HTTPException) as error:
                studio.start_render(studio.RenderRequest(source=source, scene="Demo"))
            self.assertEqual(error.exception.status_code, 422)
            self.assertIn("Typst", error.exception.detail)

    def test_math_snapshot_rejects_changed_or_extra_files(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            math = root / "math"
            math.mkdir()
            (math / "latex.exe").write_bytes(b"pinned native fixture")
            manifest = root / "inventory.json"
            manifest.write_text(json.dumps(bootstrap.inventory(math)))
            bootstrap.verify_math_payload(math, manifest)
            (math / "latex.exe").write_bytes(b"changed native fixture")
            with self.assertRaisesRegex(RuntimeError, "failed verification"):
                bootstrap.verify_math_payload(math, manifest)


if __name__ == "__main__":
    unittest.main()
