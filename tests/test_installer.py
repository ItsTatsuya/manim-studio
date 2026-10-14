"""Exercise shipped Inno file flags against an existing read-only payload."""

import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def inno_compiler():
    candidates = [ROOT / ".build-cache/inno/ISCC.exe"]
    if executable := shutil.which("ISCC.exe"):
        candidates.append(Path(executable))
    for variable in ("ProgramFiles(x86)", "ProgramFiles"):
        if directory := os.environ.get(variable):
            candidates.append(Path(directory) / "Inno Setup 6/ISCC.exe")
    return next((path for path in candidates if path.is_file()), None)


@unittest.skipUnless(os.name == "nt", "Requires Windows and Inno Setup")
class InstallerTests(unittest.TestCase):
    def test_evergreen_preflight_blocks_cleanup_on_failure(self):
        compiler = inno_compiler()
        if compiler is None:
            self.skipTest("Inno Setup compiler unavailable")
        production = (ROOT / "packaging/evergreen.iss").read_text()
        install_code = production[
            production.index("function EnsureEvergreenWebView2") :
        ]
        for script in ("studio.iss", "online.iss"):
            self.assertIn(
                "EnsureEvergreenWebView2(", (ROOT / "packaging" / script).read_text()
            )
        with tempfile.TemporaryDirectory(prefix="manim-evergreen-") as directory:
            base = Path(directory).resolve()
            for available in (False, True):
                with self.subTest(available=available):
                    case = base / str(available)
                    install = case / "installed"
                    browser = install / "webview2/old-browser.txt"
                    browser.parent.mkdir(parents=True)
                    browser.write_bytes(b"old browser")
                    user = install / "UserData/preferences.json"
                    user.parent.mkdir()
                    user.write_bytes(b"saved work")
                    target = install / "runtime.txt"
                    target.write_bytes(b"old runtime")
                    source = case / "runtime.txt"
                    source.write_bytes(b"new runtime")
                    fixture = case / "fixture.iss"
                    fixture.write_text(
                        "[Setup]\nAppName=Studio Evergreen regression\nAppVersion=1.0.0\n"
                        f"DefaultDirName={install}\nOutputDir={case}\nOutputBaseFilename=fixture\n"
                        "PrivilegesRequired=lowest\nUninstallable=no\nCreateUninstallRegKey=no\n"
                        "DisableDirPage=yes\nDisableProgramGroupPage=yes\nDirExistsWarning=no\nCloseApplications=no\n"
                        '[InstallDelete]\nType: filesandordirs; Name: "{app}\\webview2"\n'
                        f'[Files]\nSource: "{source}"; DestDir: "{{app}}"; Flags: ignoreversion\n'
                        "[Code]\nfunction HasEvergreenWebView2: Boolean;\nbegin\n"
                        f"  Result := {str(available)};\nend;\n"
                        + install_code
                        + "\nfunction PrepareToInstall(var NeedsRestart: Boolean): String;\nbegin\n"
                        "  Result := EnsureEvergreenWebView2('unavailable-installer.exe');\nend;\n",
                        encoding="utf-8",
                    )
                    compiled = subprocess.run(
                        [str(compiler), "/Q", str(fixture)],
                        capture_output=True,
                        text=True,
                        timeout=60,
                        creationflags=subprocess.CREATE_NO_WINDOW,
                    )
                    self.assertEqual(
                        compiled.returncode, 0, compiled.stdout + compiled.stderr
                    )
                    log = case / "install.log"
                    result = subprocess.run(
                        [
                            str(case / "fixture.exe"),
                            "/VERYSILENT",
                            "/SUPPRESSMSGBOXES",
                            "/NORESTART",
                            f"/LOG={log}",
                        ],
                        timeout=60,
                        creationflags=subprocess.CREATE_NO_WINDOW,
                    )
                    self.assertEqual(
                        result.returncode,
                        0 if available else 7,
                        log.read_text(errors="replace"),
                    )
                    self.assertEqual(browser.exists(), not available)
                    self.assertEqual(
                        target.read_bytes(),
                        b"new runtime" if available else b"old runtime",
                    )
                    self.assertEqual(user.read_bytes(), b"saved work")

    def test_path_preflight_preserves_existing_install_at_rejected_boundary(self):
        compiler = inno_compiler()
        if compiler is None:
            self.skipTest("Inno Setup compiler unavailable")
        for script in ("studio.iss", "online.iss"):
            code = (ROOT / "packaging" / script).read_text()
            self.assertIn('#include "installer-paths.iss"', code)
            self.assertIn(
                "Result := ValidateInstallerPath({#PayloadPathLength});", code
            )
        self.assertEqual(
            (ROOT / "packaging/online.iss")
            .read_text()
            .count("Result := ValidatePreparedPaths;"),
            2,
        )
        with tempfile.TemporaryDirectory(prefix="manim-path-") as directory:
            base = Path(directory).resolve()
            for external in (False, True):
                for length, state in (
                    (243, "ready"),
                    (244, "ready"),
                    (243, "missing"),
                    (243, "empty"),
                ):
                    if not external and state != "ready":
                        continue
                    with self.subTest(external=external, length=length, state=state):
                        case = base / f"{external}-{length}-{state}"
                        source = case / "source"
                        install = case / "installed"
                        install.mkdir(parents=True)
                        remaining = length - len(str(install)) - len("\\runtime.txt")
                        components = []
                        while remaining > 0:
                            count = min(30, remaining - 1)
                            if remaining - count - 1 == 1:
                                count -= 1
                            self.assertGreater(count, 0)
                            components.append("a" * count)
                            remaining -= count + 1
                        relative = Path(*components) / "runtime.txt"
                        target = install / relative
                        self.assertEqual(len(str(target)), length)
                        target.parent.mkdir(parents=True)
                        target.write_bytes(b"old shipped runtime")
                        target.chmod(stat.S_IREAD)
                        new_file = source / relative
                        new_file.parent.mkdir(parents=True)
                        new_file.write_bytes(b"new shipped runtime")
                        user_data = install / "UserData"
                        user_data.mkdir()
                        sentinel = user_data / "preferences.json"
                        sentinel.write_bytes(b"existing user preferences")
                        obsolete = install / "obsolete.txt"
                        obsolete.write_bytes(b"old app file")
                        code = f'#include "{ROOT / "packaging/installer-paths.iss"}"\n'
                        code += "function PrepareToInstall(var NeedsRestart: Boolean): String;\nbegin\n"
                        if external:
                            prepared = "{tmp}\\prepared\\" + str(relative)
                            code += (
                                "  Result := ValidateInstallerPath(1);\n"
                                "  if Result <> '' then exit;\n"
                            )
                            if state == "ready":
                                code += (
                                    f"  ForceDirectories(ExtractFileDir(ExpandConstant('{prepared}')));\n"
                                    f"  if not FileCopy('{new_file}', ExpandConstant('{prepared}'), False) then RaiseException('Fixture preparation failed');\n"
                                )
                            elif state == "empty":
                                code += "  ForceDirectories(ExpandConstant('{tmp}\\prepared'));\n"
                            code += "  Result := ValidatePreparedPaths;\n"
                            file_entry = (
                                f'Source: "{{tmp}}\\prepared\\{relative}"; '
                                f'DestDir: "{{app}}\\{relative.parent}"; '
                                "Flags: external ignoreversion overwritereadonly\n"
                            )
                        else:
                            code += f"  Result := ValidateInstallerPath({len(str(relative))});\n"
                            file_entry = (
                                f'Source: "{new_file}"; DestDir: "{{app}}\\{relative.parent}"; '
                                "Flags: ignoreversion overwritereadonly\n"
                            )
                        code += "end;\n"
                        script = case / "fixture.iss"
                        script.write_text(
                            "[Setup]\nAppName=Manim Studio path regression\nAppVersion=1.0.0\n"
                            f"DefaultDirName={install}\nPrivilegesRequired=lowest\n"
                            "Uninstallable=no\nCreateUninstallRegKey=no\nDisableDirPage=yes\n"
                            "DisableProgramGroupPage=yes\nDirExistsWarning=no\nCloseApplications=no\n"
                            f"Compression=none\nOutputDir={case}\nOutputBaseFilename=fixture\n"
                            '[InstallDelete]\nType: files; Name: "{app}\\obsolete.txt"\n'
                            "[Files]\n" + file_entry + "[Code]\n" + code,
                            encoding="utf-8",
                        )
                        compiled = subprocess.run(
                            [str(compiler), "/Q", str(script)],
                            capture_output=True,
                            text=True,
                            timeout=60,
                            creationflags=subprocess.CREATE_NO_WINDOW,
                        )
                        self.assertEqual(
                            compiled.returncode, 0, compiled.stdout + compiled.stderr
                        )
                        log = case / "install.log"
                        result = subprocess.run(
                            [
                                str(case / "fixture.exe"),
                                "/VERYSILENT",
                                "/SUPPRESSMSGBOXES",
                                "/NORESTART",
                                f"/LOG={log}",
                            ],
                            timeout=60,
                            creationflags=subprocess.CREATE_NO_WINDOW,
                        )
                        accepted = length == 243 and state == "ready"
                        self.assertEqual(
                            result.returncode,
                            0 if accepted else 7,
                            log.read_text(errors="replace"),
                        )
                        self.assertEqual(
                            target.read_bytes(),
                            b"new shipped runtime"
                            if accepted
                            else b"old shipped runtime",
                        )
                        self.assertEqual(obsolete.exists(), not accepted)
                        self.assertEqual(
                            sentinel.read_bytes(), b"existing user preferences"
                        )
                        if not accepted:
                            self.assertIn(
                                "installation folder is too long"
                                if state == "ready"
                                else "Prepared rendering tools",
                                log.read_text(errors="replace"),
                            )
                        target.chmod(stat.S_IWRITE)

    def test_silent_upgrade_replaces_read_only_payload_and_preserves_user_data(self):
        compiler = inno_compiler()
        if compiler is None:
            self.skipTest("Inno Setup compiler unavailable")
        entries = []
        for script in ("studio.iss", "online.iss"):
            for line in (ROOT / "packaging" / script).read_text().splitlines():
                if 'DestDir: "{app}";' in line and "Flags:" in line:
                    entries.append((script, line.split("Flags:", 1)[1].strip()))
        self.assertEqual(len(entries), 3)
        with tempfile.TemporaryDirectory(prefix="manim-installer-") as directory:
            base = Path(directory).resolve()
            for index, (script, production_flags) in enumerate(entries):
                with self.subTest(script=script, flags=production_flags):
                    case = base / str(index)
                    case.mkdir()
                    source = case / "source"
                    source.mkdir()
                    (source / "runtime.txt").write_bytes(b"new shipped runtime")
                    install = case / "installed"
                    install.mkdir()
                    target = install / "runtime.txt"
                    user_data = install / "UserData"
                    user_data.mkdir()
                    sentinel = user_data / "preferences.json"
                    sentinel.write_bytes(b"existing user preferences")
                    for fixed in (False, True):
                        flags = production_flags
                        if not fixed:
                            flags = re.sub(r"\boverwritereadonly\s*", "", flags)
                        if target.exists():
                            target.chmod(stat.S_IWRITE)
                        target.write_bytes(b"old shipped runtime")
                        target.chmod(stat.S_IREAD)
                        name = "fixed" if fixed else "negative-control"
                        installer_script = case / (name + ".iss")
                        installer_script.write_text(
                            "[Setup]\n"
                            "AppName=Manim Studio installer regression\n"
                            "AppVersion=1.0.0\n"
                            f"DefaultDirName={install}\n"
                            "PrivilegesRequired=lowest\n"
                            "Uninstallable=no\n"
                            "CreateUninstallRegKey=no\n"
                            "DisableDirPage=yes\n"
                            "DisableProgramGroupPage=yes\n"
                            "DirExistsWarning=no\n"
                            "CloseApplications=no\n"
                            "Compression=none\n"
                            f"OutputDir={case}\n"
                            f"OutputBaseFilename={name}\n"
                            "[Files]\n"
                            f'Source: "{source}\\*"; DestDir: "{{app}}"; '
                            f"Flags: {flags}\n",
                            encoding="utf-8",
                        )
                        compiled = subprocess.run(
                            [str(compiler), "/Q", str(installer_script)],
                            capture_output=True,
                            text=True,
                            timeout=60,
                            creationflags=subprocess.CREATE_NO_WINDOW,
                        )
                        self.assertEqual(
                            compiled.returncode, 0, compiled.stdout + compiled.stderr
                        )
                        log = case / (name + ".log")
                        installed = subprocess.run(
                            [
                                str(case / (name + ".exe")),
                                "/VERYSILENT",
                                "/SUPPRESSMSGBOXES",
                                "/NORESTART",
                                "/NOICONS",
                                f"/DIR={install}",
                                f"/LOG={log}",
                            ],
                            capture_output=True,
                            text=True,
                            timeout=60,
                            creationflags=subprocess.CREATE_NO_WINDOW,
                        )
                        self.assertEqual(
                            installed.returncode,
                            0 if fixed else 5,
                            log.read_text(errors="replace"),
                        )
                        self.assertEqual(
                            target.read_bytes(),
                            b"new shipped runtime" if fixed else b"old shipped runtime",
                        )
                        self.assertEqual(
                            bool(
                                target.stat().st_file_attributes
                                & stat.FILE_ATTRIBUTE_READONLY
                            ),
                            not fixed,
                        )
                        self.assertEqual(
                            sentinel.read_bytes(), b"existing user preferences"
                        )
                    target.chmod(stat.S_IWRITE)
