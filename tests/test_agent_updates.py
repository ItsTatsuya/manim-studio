"""Agent CLI updates: versions, install detection, deferral and failures.

No test reaches the network or changes an installed CLI: the registry lookup,
the version probe and the update command are all fakes.
"""

import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import agent_updates
import agents
import studio

FAKE = Path(__file__).with_name("fake_agent_cli.py")


class VersionTests(unittest.TestCase):
    def test_versions_compare_numerically_and_prereleases_first(self):
        compare = agent_updates.compare_versions
        self.assertEqual(compare("0.10.0", "0.9.9"), 1)
        self.assertEqual(compare("2.1.311 (Claude Code)", "2.1.311"), 0)
        self.assertEqual(compare("codex-cli 0.164.0", "0.165.0"), -1)
        self.assertEqual(compare("1.0.0-beta.2", "1.0.0"), -1)
        self.assertEqual(compare("1.0.0-beta.10", "1.0.0-beta.2"), 1)
        self.assertEqual(compare("1.0.0-alpha", "1.0.0-1"), 1)
        self.assertIsNone(compare("unknown", "1.0.0"))
        self.assertIsNone(compare("1.0.0", None))

    def test_registry_response_is_validated(self):
        class Response:
            def __init__(
                self, body, url="https://registry.npmjs.org/@openai%2Fcodex/latest"
            ):
                self.body, self.url = body, url

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

            def geturl(self):
                return self.url

            def read(self, limit):
                return self.body[:limit]

        with patch(
            "urllib.request.urlopen",
            return_value=Response(json.dumps({"version": "0.170.1"}).encode()),
        ) as opened:
            self.assertEqual(agent_updates.latest_version("codex"), "0.170.1")
        self.assertIn("registry.npmjs.org", opened.call_args[0][0].full_url)
        with patch(
            "urllib.request.urlopen",
            return_value=Response(json.dumps({"version": "latest"}).encode()),
        ):
            with self.assertRaises(agent_updates.AgentUpdateError):
                agent_updates.latest_version("claude")
        with patch(
            "urllib.request.urlopen",
            return_value=Response(b"{}", url="https://example.com/"),
        ):
            with self.assertRaises(agent_updates.AgentUpdateError):
                agent_updates.latest_version("claude")


class InstallMethodTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_windows_npm_prefix_updates_that_prefix(self):
        prefix = self.root / "npm"
        package = prefix / "node_modules" / "@openai" / "codex"
        package.mkdir(parents=True)
        (package / "package.json").write_text("{}", encoding="utf-8")
        binary = prefix / "codex.cmd"
        binary.write_text("", encoding="utf-8")
        method = agent_updates.install_method("codex", str(binary), npm="npm.cmd")
        self.assertEqual(method["method"], "npm")
        self.assertEqual(
            method["command"],
            [
                "npm.cmd",
                "install",
                "--global",
                "--prefix",
                str(prefix),
                "--no-fund",
                "--no-audit",
                "@openai/codex@latest",
            ],
        )

    def test_posix_npm_prefix_and_missing_npm(self):
        prefix = self.root / "usr"
        package = prefix / "lib" / "node_modules" / "@anthropic-ai" / "claude-code"
        package.mkdir(parents=True)
        (package / "package.json").write_text("{}", encoding="utf-8")
        (prefix / "bin").mkdir()
        binary = prefix / "bin" / "claude"
        binary.write_text("", encoding="utf-8")
        with patch("agent_updates.shutil.which", return_value=None):
            method = agent_updates.install_method("claude", str(binary))
        self.assertEqual(method["method"], "npm")
        self.assertEqual(method["command"], [])
        self.assertIn("npm was not found", method["problem"])
        method = agent_updates.install_method("claude", str(binary), npm="npm")
        self.assertIn(str(prefix), method["command"])
        self.assertEqual(method["command"][-1], "@anthropic-ai/claude-code@latest")

    def test_native_installs_use_their_own_update_command(self):
        home = self.root / "home"
        (home / ".local" / "share" / "claude").mkdir(parents=True)
        (home / ".local" / "bin").mkdir(parents=True)
        claude = home / ".local" / "bin" / "claude.exe"
        claude.write_text("", encoding="utf-8")
        local = self.root / "local"
        codex = local / "Programs" / "OpenAI" / "Codex" / "bin" / "codex.exe"
        codex.parent.mkdir(parents=True)
        codex.write_text("", encoding="utf-8")
        with (
            patch("agent_updates.Path.home", return_value=home),
            patch.dict(os.environ, {"LOCALAPPDATA": str(local)}),
        ):
            self.assertEqual(
                agent_updates.install_method("claude", str(claude)),
                {"method": "native", "command": [str(claude), "update"]},
            )
            if os.name == "nt":
                self.assertEqual(
                    agent_updates.install_method("codex", str(codex)),
                    {"method": "native", "command": [str(codex), "update"]},
                )

    def test_custom_binaries_are_managed_externally(self):
        custom = self.root / "tools" / "claude.exe"
        custom.parent.mkdir()
        custom.write_text("", encoding="utf-8")
        with patch("agent_updates.Path.home", return_value=self.root / "home"):
            self.assertEqual(
                agent_updates.install_method("claude", str(custom))["method"],
                "external",
            )
            self.assertEqual(
                agent_updates.install_method("codex", "fixture-cli")["method"],
                "external",
            )
        self.assertEqual(
            agent_updates.install_method("codex", None)["method"], "missing"
        )


class SettingsTests(unittest.TestCase):
    def test_auto_update_defaults_on_and_old_settings_migrate(self):
        self.assertTrue(agents.clean_settings({})["auto_update_agents"])
        legacy = {"access": "edits", "auto_render": False}
        self.assertTrue(agents.clean_settings(legacy)["auto_update_agents"])
        self.assertFalse(
            agents.clean_settings({"auto_update_agents": False})["auto_update_agents"]
        )
        self.assertTrue(
            agents.clean_settings({"auto_update_agents": "no"})["auto_update_agents"]
        )


class UpdaterCase(unittest.TestCase):
    """An updater with a fake manager, fake versions and a fake update command."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.events = []
        self.latest = {"codex": "0.200.0", "claude": "2.0.0"}
        self.versions = {"codex": "0.100.0", "claude": "2.0.0"}
        self.commands = []
        self.exit_code = 0
        patches = [
            patch.object(
                agents, "launch_command", lambda binary: [sys.executable, str(FAKE)]
            ),
            patch.object(
                agents, "resolve_binary", lambda provider, configured="": provider
            ),
            patch.object(agents, "publish", lambda kind, payload: None),
        ]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)
        self.manager = agents.AgentManager(
            self.root,
            python=sys.executable,
            app_root=Path(studio.APP_ROOT),
            latex=lambda: False,
        )
        self.addCleanup(self.manager.shutdown)
        self.updater = agent_updates.AgentUpdater(
            lambda: self.manager,
            publish=lambda kind, payload: self.events.append((kind, payload)),
            fetch_latest=self.fetch_latest,
            method=self.method,
            installed_version=lambda binary: self.versions[binary],
            automatic=False,
        )
        self.updater._execute = self.execute
        self.addCleanup(self.updater.shutdown)

    def fetch_latest(self, provider):
        value = self.latest[provider]
        if isinstance(value, Exception):
            raise value
        return value

    def method(self, provider, binary):
        return {"method": "npm", "command": ["npm", "install", provider]}

    def execute(self, command):
        self.commands.append(command)
        if self.exit_code == 0:
            self.versions[command[-1]] = self.latest[command[-1]]
        return self.exit_code, "npm output tail"

    def wait_for(self, predicate, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.05)
        self.fail("Condition was not reached")


class UpdaterTests(UpdaterCase):
    def test_check_marks_available_and_persists(self):
        self.manager.update_settings({"auto_update_agents": False})
        self.updater.check_all()
        state = self.updater.snapshot()
        self.assertEqual(state["providers"]["codex"]["status"], "available")
        self.assertTrue(state["providers"]["codex"]["available"])
        self.assertEqual(state["providers"]["claude"]["status"], "current")
        self.assertFalse(state["auto"])
        self.assertEqual(self.updater.pending, set())  # Auto-update is off.
        saved = json.loads(
            (self.root / ".studio" / "agent-updates.json").read_text("utf-8")
        )
        self.assertEqual(saved["providers"]["codex"]["latest"], "0.200.0")
        self.assertIsNotNone(saved["last_check"])
        self.assertTrue(any(kind == "agent.updates" for kind, _ in self.events))
        reloaded = agent_updates.AgentUpdater(lambda: self.manager, automatic=False)
        reloaded.load()
        self.assertEqual(reloaded.last_check, saved["last_check"])
        self.assertTrue(reloaded.snapshot()["providers"]["codex"]["available"])

    def test_auto_update_runs_after_check(self):
        self.updater.check_all()
        self.assertEqual(self.updater.pending, {"codex"})
        self.updater._run_pending()
        self.assertEqual(self.commands, [["npm", "install", "codex"]])
        entry = self.updater.snapshot()["providers"]["codex"]
        self.assertEqual((entry["status"], entry["installed"]), ("current", "0.200.0"))
        self.assertEqual(entry["output"], "npm output tail")
        self.assertEqual(self.updater.pending, set())
        self.assertEqual(self.manager.updating, set())

    def test_offline_and_failed_updates_are_statuses(self):
        self.latest["claude"] = OSError("network is unreachable")
        self.updater.check_all()
        claude = self.updater.snapshot()["providers"]["claude"]
        self.assertEqual(claude["status"], "error")
        self.assertIn("network is unreachable", claude["message"])
        self.exit_code = 243
        self.updater._execute = lambda command: (
            243,
            "npm ERR! EACCES permission denied",
        )
        self.updater._run_pending()
        codex = self.updater.snapshot()["providers"]["codex"]
        self.assertEqual(codex["status"], "error")
        self.assertIn("exit code 243", codex["message"])
        self.assertIn("permission", codex["message"])
        self.assertIn("EACCES", codex["output"])
        self.assertEqual(self.manager.updating, set())

    def test_external_installs_are_reported_not_updated(self):
        self.updater.method = lambda provider, binary: {
            "method": "external",
            "command": [],
        }
        self.updater.check_all()
        codex = self.updater.snapshot()["providers"]["codex"]
        self.assertEqual(codex["status"], "external")
        self.assertIn("0.200.0", codex["message"])
        self.assertEqual(self.updater.pending, set())
        with self.assertRaises(agent_updates.AgentUpdateError):
            self.updater.update_now("codex")

    def test_update_waits_while_provider_turn_runs(self):
        class Busy:
            record = {"provider": "codex"}

        with self.manager.lock:
            self.manager.running["thread"] = Busy()
        self.updater.pending.add("codex")
        self.updater._run_pending()
        self.assertEqual(self.commands, [])
        codex = self.updater.snapshot()["providers"]["codex"]
        self.assertEqual(codex["status"], "waiting")
        self.assertEqual(self.updater.pending, {"codex"})
        with self.manager.lock:
            self.manager.running.clear()
        self.updater._run_pending()
        self.assertEqual(self.commands, [["npm", "install", "codex"]])
        self.assertEqual(self.updater.pending, set())

    def test_new_turns_wait_for_the_update_window(self):
        import projects

        project = projects.create(self.root / "projects", "Demo")
        thread = self.manager.create(project["id"], provider="codex")
        with self.manager.lock:
            self.manager.updating.add("codex")
        with patch.object(agents, "UPDATE_WAIT", 0.3):
            with self.assertRaisesRegex(agents.AgentError, "updating"):
                self.manager.send(project["id"], thread["id"], "Hello")
            release = threading.Timer(
                0.1, lambda: self.manager.updating.discard("codex")
            )
            release.start()
            with patch.object(agents, "UPDATE_WAIT", 10):
                turn = self.manager.send(project["id"], thread["id"], "Hello")
            release.join()
        self.assertEqual(turn["status"], "running")
        self.manager.interrupt(project["id"], thread["id"], wait=True)

    def test_background_worker_handles_manual_requests(self):
        self.manager.update_settings({"auto_update_agents": False})
        self.updater.check_now()
        self.wait_for(lambda: self.updater.snapshot()["last_check"] is not None)
        self.wait_for(lambda: not self.updater.snapshot()["checking"])
        self.updater.update_now("codex")
        self.wait_for(lambda: self.commands == [["npm", "install", "codex"]])
        with self.assertRaises(agent_updates.AgentUpdateError):
            self.updater.update_now("unknown")
        self.manager.update_settings({"providers": {"claude": {"enabled": False}}})
        with self.assertRaisesRegex(agent_updates.AgentUpdateError, "turned off"):
            self.updater.update_now("claude")

    def test_automatic_check_waits_for_interval(self):
        self.updater.automatic = True
        self.updater.started_at = time.monotonic() - agent_updates.STARTUP_DELAY - 1
        self.assertTrue(self.updater._due())
        self.updater.last_check = agent_updates._now()
        self.assertFalse(self.updater._due())
        self.manager.update_settings({"auto_update_agents": False})
        self.updater.last_check = None
        self.assertFalse(self.updater._due())


class ProcessTests(unittest.TestCase):
    def test_update_command_runs_hidden_and_captures_output(self):
        updater = agent_updates.AgentUpdater(lambda: None, automatic=False)
        code, output = updater._execute(
            [sys.executable, "-c", "print('installed'); import sys; sys.exit(3)"]
        )
        self.assertEqual(code, 3)
        self.assertEqual(output, "installed")

    def test_update_command_times_out(self):
        updater = agent_updates.AgentUpdater(lambda: None, automatic=False)
        with patch.object(agent_updates, "UPDATE_TIMEOUT", 0.5):
            code, _ = updater._execute(
                [sys.executable, "-c", "import time; time.sleep(30)"]
            )
        self.assertIsNone(code)


class EndpointTests(UpdaterCase):
    def test_endpoints_report_and_reject(self):
        with (
            patch.object(studio, "AGENT_UPDATES", self.updater),
            patch.object(studio, "AGENTS", self.manager),
        ):
            state = studio.agent_update_status()
            self.assertIn("codex", state["providers"])
            self.updater.method = lambda provider, binary: {
                "method": "external",
                "command": [],
            }
            with self.assertRaises(studio.HTTPException) as rejected:
                studio.update_agent_cli("codex")
            self.assertEqual(rejected.exception.status_code, 409)
            result = studio.update_agent_settings(
                studio.AgentSettings(auto_update_agents=False)
            )
            self.assertFalse(result["auto_update_agents"])


if __name__ == "__main__":
    unittest.main()
