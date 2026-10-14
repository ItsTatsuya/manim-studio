"""Runtime migration must fail before replacement when dependencies are missing."""

import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch
from zipfile import ZipFile

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
    def test_equation_extraction_reports_progress_and_rejects_unsafe_paths(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive = root / "math.zip"
            with ZipFile(archive, "w") as source:
                source.writestr("nested/equation.tex", b"equation fixture")
                source.writestr("font.ttf", b"font fixture")
            updates = []
            math = root / "math"
            bootstrap.extract_zip(archive, math, lambda *values: updates.append(values))
            self.assertEqual(updates[0][:2], (0, 2))
            self.assertEqual(updates[-1][:2], (2, 2))
            verified = []
            actual = bootstrap.inventory(math, lambda *values: verified.append(values))
            self.assertEqual(len(actual["files"]), 2)
            self.assertEqual(verified[-1][:2], (2, 2))
            with ZipFile(archive, "w") as source:
                source.writestr("safe.tex", b"safe")
                source.writestr("../escaped.tex", b"unsafe")
            with self.assertRaisesRegex(RuntimeError, "Unsafe archive path"):
                bootstrap.extract_zip(archive, math)
            self.assertFalse((root / "escaped.tex").exists())
            self.assertFalse((math / "safe.tex").exists())

    def test_progress_reports_slow_work_without_flooding_setup_output(self):
        updates = []
        with patch.object(bootstrap.time, "monotonic", side_effect=[0, 0, 0.1, 2, 2.1]):
            report = bootstrap.progress_reporter(
                60, 78, "Extracting", lambda *v: updates.append(v)
            )
            report(0, 100, "Starting")
            report(1, 100, "file 1")
            report(50, 100, "file 50")
            report(100, 100, "file 100")
        self.assertEqual([item[0] for item in updates], [60, 69, 78])
        self.assertIn("2s", updates[1][1])

    def test_setup_tools_stream_output_and_report_quiet_work(self):
        lines, beats = [], []
        code, tail = bootstrap.run_tool(
            [
                sys.executable,
                "-c",
                "import time\nprint('first', flush=True)\ntime.sleep(2.5)\nprint('done')",
            ],
            timeout=60,
            on_line=lines.append,
            heartbeat=beats.append,
            interval=1,
        )
        self.assertEqual((code, lines, tail), (0, ["first", "done"], ["first", "done"]))
        self.assertGreaterEqual(len(beats), 1)

    def test_setup_tool_hangs_stop_the_whole_process_tree(self):
        script = (
            "import subprocess, sys, time\n"
            "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])\n"
            "print('started', flush=True)\n"
            "time.sleep(60)\n"
        )
        started = time.monotonic()
        with self.assertRaisesRegex(RuntimeError, "did not finish"):
            bootstrap.run_tool([sys.executable, "-c", script], timeout=2)
        self.assertLess(time.monotonic() - started, 30)

    def test_helper_holding_the_output_pipe_does_not_block_setup(self):
        # The tool exits at once; its helper keeps the inherited stdout open.
        script = (
            "import subprocess, sys\n"
            "subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(40)'])\n"
            "print('tool finished', flush=True)\n"
        )
        started = time.monotonic()
        code, tail = bootstrap.run_tool([sys.executable, "-c", script], timeout=120)
        self.assertEqual((code, tail), (0, ["tool finished"]))
        self.assertLess(time.monotonic() - started, 30)

    def test_native_probe_terminates_hangs_and_reports_crashes(self):
        with self.assertRaisesRegex(RuntimeError, "timed out"):
            bootstrap.native_probe(
                [sys.executable, "-c", "import time; time.sleep(30)"],
                None,
                timeout=0.1,
            )
        with self.assertRaisesRegex(RuntimeError, r"exit code 99, 0x00000063"):
            bootstrap.native_probe(
                [sys.executable, "-c", "import os; os._exit(99)"],
                None,
            )
        self.assertEqual(
            bootstrap.native_probe(
                [sys.executable, "-c", "print('ready')"], None
            ).strip(),
            "ready",
        )

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
            with self.assertRaisesRegex(RuntimeError, "failed verification: latex.exe"):
                bootstrap.verify_math_payload(math, manifest)
            (math / "latex.exe").write_bytes(b"pinned native fixture")
            (math / "unexpected.dll").write_bytes(b"extra fixture")
            with self.assertRaisesRegex(
                RuntimeError, "failed verification: unexpected.dll"
            ):
                bootstrap.verify_math_payload(math, manifest)


if __name__ == "__main__":
    unittest.main()
