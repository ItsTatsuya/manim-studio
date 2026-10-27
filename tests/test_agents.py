"""Agent CLIs: provider checks, stream translation, turns and their API."""

import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import agents
from events import EventBus
import projects
import studio

FAKE = Path(__file__).with_name("fake_agent_cli.py")


class FakeCliCase(unittest.TestCase):
    """A data root with one project and fake provider binaries."""

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.log = self.root / "invocations.jsonl"
        self.events = []
        self.project = projects.create(self.root / "projects", "Demo")
        patches = [
            patch.object(
                agents, "launch_command", lambda binary: [sys.executable, str(FAKE)]
            ),
            patch.object(
                agents, "resolve_binary", lambda provider, configured="": "fake"
            ),
            patch.object(
                agents,
                "publish",
                lambda kind, payload: self.events.append((kind, payload)),
            ),
            patch.dict(
                os.environ,
                {
                    "FAKE_AGENT_LOG": str(self.log),
                    "CODEX_HOME": str(self.root / "codex-home"),
                },
            ),
            patch.dict(agents._CAPABILITIES, clear=True),
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
        self.addCleanup(self.temporary.cleanup)

    def invocations(self):
        if not self.log.is_file():
            return []
        return [
            json.loads(line)
            for line in self.log.read_text(encoding="utf-8").splitlines()
        ]

    def wait_idle(self, thread_id, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self.manager.lock:
                if thread_id not in self.manager.running:
                    return
            time.sleep(0.05)
        self.fail("The agent turn did not finish")


class ProviderProbeTests(FakeCliCase):
    def test_install_and_login_launch_only_known_commands(self):
        if os.name != "nt":
            self.skipTest("Windows interactive CLI setup")
        with (
            patch("agents.shutil.which", return_value="powershell.exe"),
            patch("agents.subprocess.Popen") as launch,
        ):
            for provider in agents.PROVIDERS:
                self.assertEqual(self.manager.open_install(provider), {"ok": True})
                command = launch.call_args.args[0]
                self.assertEqual(
                    command[:4], ["powershell.exe", "-NoProfile", "-NoExit", "-Command"]
                )
                self.assertIn(
                    "npm.cmd install -g @openai/codex"
                    if provider == "codex"
                    else "https://claude.ai/install.ps1",
                    command[4],
                )
                self.assertEqual(
                    launch.call_args.kwargs["creationflags"],
                    agents.subprocess.CREATE_NEW_CONSOLE,
                )
            with self.assertRaises(agents.AgentError):
                self.manager.open_install("codex; malicious")
            self.manager.open_login("claude")
            self.assertEqual(launch.call_args.args[0][-2:], ["auth", "login"])
        with patch("agents.shutil.which", return_value=None):
            with self.assertRaisesRegex(agents.AgentError, "PowerShell"):
                self.manager.open_install("codex")
        with (
            patch("agents.shutil.which", return_value="powershell.exe"),
            patch("agents.subprocess.Popen", side_effect=OSError("blocked")),
        ):
            with self.assertRaisesRegex(agents.AgentError, "blocked"):
                self.manager.open_install("claude")

    def test_probe_reports_version_auth_and_live_models(self):
        settings = agents.clean_settings({})
        codex = agents.probe("codex", settings)
        self.assertEqual((codex["status"], codex["version"]), ("ready", "1.2.3"))
        self.assertEqual(codex["auth"], {"status": "authenticated", "label": "ChatGPT"})
        self.assertEqual([model["id"] for model in codex["models"]], ["gpt-test"])
        self.assertEqual(codex["models"][0]["efforts"], ["low", "high"])
        claude = agents.probe("claude", settings)
        self.assertEqual(claude["status"], "ready")
        self.assertEqual(claude["auth"]["label"], "Claude Pro")
        self.assertIn("claude-sonnet-5-5", [model["id"] for model in claude["models"]])

    def test_signed_out_disabled_and_missing_providers(self):
        settings = agents.clean_settings({})
        with patch.dict(os.environ, {"FAKE_AGENT_AUTH": "none"}):
            for provider in agents.PROVIDERS:
                status = agents.probe(provider, settings)
                self.assertEqual(status["status"], "warning")
                self.assertEqual(status["auth"]["status"], "unauthenticated")
        settings["providers"]["claude"]["enabled"] = False
        self.assertEqual(agents.probe("claude", settings)["status"], "disabled")
        with patch.object(
            agents, "resolve_binary", lambda provider, configured="": None
        ):
            status = agents.probe("codex", agents.clean_settings({}))
        self.assertEqual((status["status"], status["installed"]), ("error", False))
        self.assertIn("not found", status["message"])

    def test_settings_are_validated_and_custom_models_listed(self):
        settings = agents.clean_settings(
            {
                "providers": {
                    "codex": {
                        "model": "bad model;rm",
                        "effort": "huge",
                        "custom_models": ["my-model", 3, "x y"],
                    },
                    "claude": {"enabled": "yes", "binary": 'C:\\Tools\\claude.exe"'},
                },
                "access": "root",
                "auto_render": False,
                "default_provider": "claude",
            }
        )
        codex = settings["providers"]["codex"]
        self.assertEqual((codex["model"], codex["effort"]), ("", "medium"))
        self.assertEqual(codex["custom_models"], ["my-model"])
        self.assertTrue(settings["providers"]["claude"]["enabled"])
        self.assertEqual(
            settings["providers"]["claude"]["binary"], "C:\\Tools\\claude.exe"
        )
        self.assertEqual((settings["access"], settings["auto_render"]), ("full", False))
        models = agents._models("codex", None, codex)
        self.assertEqual(models[-1]["id"], "my-model")

    def test_manager_refresh_publishes_statuses(self):
        snapshot = self.manager.refresh(wait=True)
        self.assertEqual(
            [item["status"] for item in snapshot["providers"]], ["ready", "ready"]
        )
        self.assertFalse(snapshot["probing"])
        self.assertEqual(self.events[-1][0], "providers")

    def test_shell_wrappers_are_hidden_from_command_rows(self):
        self.assertEqual(
            agents.display_command(
                '"C:\\\\Program Files\\\\pwsh.exe" -Command "Get-Content script.py"'
            ),
            "Get-Content script.py",
        )
        self.assertEqual(agents.display_command(["bash", "-lc", "ls"]), "bash -lc ls")


class StreamTranslationTests(unittest.TestCase):
    class Recorder:
        def __init__(self, provider):
            self.manager = type("M", (), {})()
            self.items_by_key = {}
            self.session = None
            self.error = None
            self.usage = None
            self.project_dir = Path("C:/projects/demo")
            self.record = {"provider": provider}

        def set_session(self, value):
            self.session = value

        def upsert(self, key, **fields):
            self.items_by_key.setdefault(key, {}).update(fields)

        def append_text(self, key, text):
            self.items_by_key[key]["text"] = (
                self.items_by_key[key].get("text", "") + text
            )

        def fail(self, message):
            self.error = message

    def test_codex_events(self):
        turn = self.Recorder("codex")
        stream = agents.CodexStream(turn)
        for event in [
            {
                "type": "thread.started",
                "thread_id": "01a13a52-9faf-7693-bb47-a3336205635f",
            },
            {
                "type": "item.completed",
                "item": {"id": "a", "type": "agent_message", "text": "Hi"},
            },
            {
                "type": "item.completed",
                "item": {
                    "id": "b",
                    "type": "command_execution",
                    "command": "ls",
                    "aggregated_output": "x",
                    "exit_code": 1,
                    "status": "completed",
                },
            },
            {
                "type": "item.completed",
                "item": {
                    "id": "c",
                    "type": "file_change",
                    "changes": [
                        {"path": "C:/projects/demo/script.py", "kind": "update"}
                    ],
                    "status": "completed",
                },
            },
            {
                "type": "item.completed",
                "item": {
                    "id": "d",
                    "type": "todo_list",
                    "items": [{"text": "Plan", "completed": True}],
                },
            },
            {"type": "error", "message": "Reconnecting... 1/5"},
            {"type": "turn.completed", "usage": {"input_tokens": 3}},
        ]:
            stream.handle(event)
        items = turn.items_by_key
        self.assertEqual(turn.session, "01a13a52-9faf-7693-bb47-a3336205635f")
        self.assertEqual(
            items["codex:a"], {"kind": "assistant", "text": "Hi", "streaming": False}
        )
        self.assertEqual(items["codex:b"]["status"], "failed")
        self.assertEqual(
            items["codex:c"]["files"], [{"path": "script.py", "kind": "update"}]
        )
        self.assertEqual(items["codex:d"]["entries"], [{"text": "Plan", "done": True}])
        self.assertIsNone(turn.error)
        self.assertEqual(turn.usage, {"input_tokens": 3})
        stream.handle({"type": "turn.failed", "error": {"message": "quota"}})
        self.assertEqual(turn.error, "quota")

    def test_claude_partial_text_and_tool_results(self):
        turn = self.Recorder("claude")
        stream = agents.ClaudeStream(turn)
        stream.handle(
            {
                "type": "system",
                "subtype": "init",
                "session_id": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            }
        )
        stream.handle(
            {
                "type": "stream_event",
                "event": {"type": "message_start", "message": {"id": "m1"}},
            }
        )
        stream.handle(
            {
                "type": "stream_event",
                "event": {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {"type": "text"},
                },
            }
        )
        stream.handle(
            {
                "type": "stream_event",
                "event": {
                    "type": "content_block_delta",
                    "index": 1,
                    "delta": {"type": "text_delta", "text": "Hel"},
                },
            }
        )
        self.assertEqual(turn.items_by_key["claude:m1:text:0"]["text"], "Hel")
        stream.handle(
            {
                "type": "assistant",
                "message": {"id": "m1", "content": [{"type": "text", "text": "Hello"}]},
            }
        )
        self.assertEqual(
            turn.items_by_key["claude:m1:text:0"],
            {"kind": "assistant", "text": "Hello", "streaming": False},
        )
        stream.handle(
            {
                "type": "assistant",
                "message": {
                    "id": "m1",
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "t1",
                            "name": "Edit",
                            "input": {"file_path": "C:/projects/demo/narration.txt"},
                        }
                    ],
                },
            }
        )
        stream.handle(
            {
                "type": "assistant",
                "message": {
                    "id": "m1",
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "t2",
                            "name": "PowerShell",
                            "input": {"command": "python check.py"},
                        }
                    ],
                },
            }
        )
        stream.handle(
            {
                "type": "assistant",
                "parent_tool_use_id": "t9",
                "message": {
                    "id": "sub",
                    "content": [{"type": "text", "text": "ignored"}],
                },
            }
        )
        stream.handle(
            {
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "t2",
                            "content": "boom",
                            "is_error": True,
                        }
                    ]
                },
            }
        )
        stream.handle(
            {
                "type": "user",
                "message": {
                    "content": [
                        {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}
                    ]
                },
            }
        )
        items = turn.items_by_key
        self.assertEqual(
            items["claude:tool:t1"]["files"],
            [{"path": "narration.txt", "kind": "update"}],
        )
        self.assertEqual(items["claude:tool:t1"]["status"], "completed")
        self.assertEqual(
            (items["claude:tool:t2"]["status"], items["claude:tool:t2"]["detail"]),
            ("failed", "boom"),
        )
        self.assertNotIn("claude:sub:text:0", items)
        stream.handle(
            {
                "type": "result",
                "subtype": "error_max_turns",
                "is_error": True,
                "result": "Too many turns",
            }
        )
        self.assertEqual(turn.error, "Too many turns")


class TurnTests(FakeCliCase):
    def start(self, provider, text="Make an intro", **options):
        thread = self.manager.create(self.project["id"], provider=provider, **options)
        self.manager.send(self.project["id"], thread["id"], text)
        self.wait_idle(thread["id"])
        return self.manager.view(self.project["id"], thread["id"])

    def test_codex_turn_writes_project_and_resumes_session(self):
        thread = self.start("codex", model="gpt-test", effort="high")
        self.assertEqual(thread["status"], "completed")
        self.assertEqual(thread["title"], "Make an intro")
        kinds = [item["kind"] for item in thread["items"]]
        self.assertEqual(
            kinds, ["user", "reasoning", "command", "file_change", "assistant"]
        )
        command = thread["items"][2]
        self.assertEqual(
            (command["title"], command["status"]),
            ("python agent_check.py", "completed"),
        )
        self.assertEqual(
            thread["items"][3]["files"], [{"path": "script.py", "kind": "add"}]
        )
        turn = thread["turns"][-1]
        self.assertEqual(
            turn["changed"], ["assets/dot.svg", "narration.txt", "script.py"]
        )
        stored = projects.read(self.root / "projects", self.project["id"])
        self.assertIn("class Intro", stored["source"])
        self.assertTrue(stored["narration"]["enabled"])
        first = self.invocations()[0]
        self.assertEqual(first["argv"][:2], ["exec", "--json"])
        self.assertIn("--dangerously-bypass-approvals-and-sandbox", first["argv"])
        self.assertIn('model_reasoning_effort="high"', first["argv"])
        self.assertEqual(first["argv"][first["argv"].index("-m") + 1], "gpt-test")
        self.assertEqual(
            Path(first["cwd"]).resolve(), (self.root / "projects" / "Demo").resolve()
        )
        self.assertIn("<studio_instructions>", first["prompt"])
        self.assertIn("agent_check.py", first["prompt"])
        self.assertIn("never use Tex or MathTex", first["prompt"])
        self.assertTrue(first["prompt"].endswith("Make an intro"))
        changed = [
            payload for kind, payload in self.events if kind == "project.changed"
        ]
        self.assertEqual(changed[-1]["files"], turn["changed"])

        self.manager.send(
            self.project["id"], thread["id"], "Make it blue", context="Render log: ok"
        )
        self.wait_idle(thread["id"])
        second = self.invocations()[1]
        self.assertEqual(second["argv"][:3], ["exec", "resume", "--json"])
        self.assertEqual(
            second["argv"][-2:], ["11111111-2222-3333-4444-555555555555", "-"]
        )
        self.assertNotIn("<studio_instructions>", second["prompt"])
        self.assertTrue(second["prompt"].startswith("Render log: ok"))
        reloaded = agents.AgentManager(
            self.root, python=sys.executable, app_root=self.root
        )
        self.assertEqual(
            len(reloaded.view(self.project["id"], thread["id"])["turns"]), 2
        )
        with self.assertRaises(agents.AgentError):
            self.manager.update(self.project["id"], thread["id"], provider="claude")

    def test_claude_turn_streams_text_and_uses_system_prompt_file(self):
        with patch.dict(self.manager.settings, {"access": "edits"}):
            thread = self.start("claude", model="haiku", effort="low")
        self.assertEqual(thread["status"], "completed")
        assistant = [item for item in thread["items"] if item["kind"] == "assistant"]
        self.assertEqual(
            [item["text"] for item in assistant], ["Writing the scene.", "Done."]
        )
        self.assertFalse(any(item.get("streaming") for item in thread["items"]))
        tools = {
            item["kind"]: item
            for item in thread["items"]
            if item["kind"] in ("file_change", "command")
        }
        self.assertEqual(tools["command"]["detail"], "OK: ready")
        argv = self.invocations()[0]["argv"]
        self.assertEqual(argv[argv.index("--permission-mode") + 1], "acceptEdits")
        self.assertIn("--allowedTools", argv)
        self.assertEqual(argv[argv.index("--model") + 1], "haiku")
        self.assertNotIn("<studio_instructions>", self.invocations()[0]["prompt"])
        deltas = [
            payload["item"]["text"]
            for kind, payload in self.events
            if kind == "thread.item" and payload["item"].get("streaming")
        ]
        self.assertIn("Writing the scene", deltas)
        self.manager.send(self.project["id"], thread["id"], "again")
        self.wait_idle(thread["id"])
        argv = self.invocations()[1]["argv"]
        self.assertEqual(
            argv[argv.index("--resume") + 1], "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        )

    def test_claude_sessions_load_only_studio_tools_and_context(self):
        for access in ("full", "edits"):
            with patch.dict(self.manager.settings, {"access": access}):
                thread = self.start("claude")
            self.assertIsNone(thread["turns"][-1]["tokens"])  # Fake has no usage.
            call = self.invocations()[-1]
            argv = call["argv"]
            self.assertEqual(
                argv[argv.index("--tools") + 1],
                "Read,Write,Edit,MultiEdit,Glob,Grep,Bash,PowerShell",
            )
            self.assertIn("--strict-mcp-config", argv)
            self.assertIn("--disable-slash-commands", argv)
            self.assertEqual(argv[argv.index("--setting-sources") + 1], "user")
            self.assertEqual(
                json.loads(call["files"]["--mcp-config"]), {"mcpServers": {}}
            )
            self.assertEqual(
                json.loads(call["files"]["--settings"]), {"disableAllHooks": True}
            )
            self.assertEqual(
                call["env"],
                {
                    "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1",
                    "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
                },
            )
            # Access modes keep their permission semantics.
            self.assertEqual(
                argv[argv.index("--permission-mode") + 1],
                "bypassPermissions" if access == "full" else "acceptEdits",
            )
            self.assertEqual("--allowedTools" in argv, access == "edits")

    def test_codex_sessions_switch_off_unused_features_and_mcp_servers(self):
        home = self.root / "codex-home"
        home.mkdir()
        (home / "config.toml").write_text(
            '[mcp_servers.docs]\ncommand = "docs"\n'
            '[mcp_servers.broken]\ncommand = "x"\n'
            '[mcp_servers."odd.name"]\ncommand = "y"\n',
            encoding="utf-8",
        )
        thread = self.start("codex")
        argv = self.invocations()[0]["argv"]
        overrides = [argv[i + 1] for i, part in enumerate(argv) if part == "-c"]
        for value in (
            'web_search="disabled"',
            "project_doc_max_bytes=0",
            "features.apps=false",
            "features.multi_agent=false",
            "mcp_servers.docs.enabled=false",
        ):
            self.assertIn(value, overrides)
        # Removed features, rejected values and unsafe names are left out.
        self.assertFalse(
            any(
                "web_search_request" in value or "broken" in value or "odd" in value
                for value in overrides
            )
        )
        self.assertNotIn("features.shell_tool=false", overrides)
        self.assertEqual(
            thread["turns"][-1]["tokens"], {"input": 10, "cached": 0, "output": 5}
        )
        # Resumed turns get the same overrides.
        self.manager.send(self.project["id"], thread["id"], "again")
        self.wait_idle(thread["id"])
        resumed = self.invocations()[1]["argv"]
        self.assertEqual(resumed[:2], ["exec", "resume"])
        self.assertIn("features.apps=false", resumed)

    def test_older_clis_get_no_unadvertised_options(self):
        with patch.dict(os.environ, {"FAKE_AGENT_LEGACY": "1"}):
            self.start("claude")
            self.start("codex")
        claude, codex = (call["argv"] for call in self.invocations())
        for flag in ("--tools", "--strict-mcp-config", "--settings"):
            self.assertNotIn(flag, claude)
        self.assertIn("--append-system-prompt-file", claude)
        self.assertFalse(
            any("features." in part or "web_search" in part for part in codex)
        )

    def test_token_usage_is_normalised_for_both_clis(self):
        self.assertEqual(
            agents.token_usage(
                {
                    "usage": {
                        "input_tokens": 12,
                        "cache_creation_input_tokens": 300,
                        "cache_read_input_tokens": 9000,
                        "output_tokens": 450,
                    },
                    "total_cost_usd": 0.1,
                }
            ),
            {"input": 9312, "cached": 9000, "output": 450},
        )
        self.assertEqual(
            agents.token_usage(
                {"input_tokens": 5000, "cached_input_tokens": 4000, "output_tokens": 7}
            ),
            {"input": 5000, "cached": 4000, "output": 7},
        )
        self.assertIsNone(agents.token_usage(None))
        self.assertIsNone(agents.token_usage({"usage": {}}))

    def test_failures_are_reported_in_the_timeline(self):
        thread = self.start("codex", text="FAIL please")
        self.assertEqual(thread["status"], "failed")
        self.assertEqual(thread["items"][-1]["kind"], "error")
        self.assertIn("model is unavailable", thread["items"][-1]["text"])
        thread = self.start("claude", text="FAIL please")
        self.assertEqual(thread["turns"][-1]["error"], "Ran out of turns")

    def test_limit_failures_get_stable_error_codes(self):
        cases = {
            "You've hit your limit · resets 3pm": ("usage_limit", None),
            "429 Too Many Requests: rate limit exceeded": ("rate_limited", 429),
            "Error: prompt is too long: 250000 tokens": ("context_too_long", None),
            "The model `gpt-9` does not exist": ("model_unavailable", None),
            "401 Unauthorized": ("auth_required", 401),
            "API Error: 529 overloaded_error": ("overloaded", 529),
            "getaddrinfo ENOTFOUND api.anthropic.com": ("network_error", None),
            "something unexpected": ("agent_error", None),
        }
        for message, (code, status) in cases.items():
            with self.subTest(message=message):
                info = agents.classify_error(message)
                self.assertEqual(info["code"], code)
                self.assertEqual(info.get("http_status"), status)
        self.assertEqual(
            agents.classify_error("Usage limit reached. Try again in 4 hours.")[
                "reset"
            ],
            "Try again in 4 hours",
        )

    def test_failed_turn_records_its_error_code(self):
        thread = self.start("codex", text="FAIL please")
        item = thread["items"][-1]
        self.assertEqual(item["kind"], "error")
        self.assertEqual(item["code"], thread["turns"][-1]["error_code"])
        self.assertEqual(item["code"], "model_unavailable")
        self.assertIn("title", item)

    def test_fork_copies_history_and_replays_it_once(self):
        thread = self.start("codex", model="gpt-test", effort="high")
        pid = self.project["id"]
        fork = self.manager.fork(pid, thread["id"], title="Try blue")
        self.assertNotEqual(fork["id"], thread["id"])
        self.assertEqual(fork["title"], "Try blue")
        self.assertEqual(fork["forked_from"]["thread"], thread["id"])
        self.assertEqual(
            [item["kind"] for item in fork["items"]],
            [item["kind"] for item in thread["items"]],
        )
        self.assertEqual((fork["model"], fork["effort"]), ("gpt-test", "high"))
        self.assertFalse(fork["locked"])
        self.manager.send(pid, fork["id"], "Make it blue")
        self.wait_idle(fork["id"])
        prompt = self.invocations()[-1]["prompt"]
        self.assertIn("<forked_conversation>", prompt)
        self.assertIn("Make an intro", prompt)
        self.assertTrue(prompt.endswith("Make it blue"))
        # The context is replayed on the first message only; the source is untouched.
        self.manager.send(pid, fork["id"], "And bigger")
        self.wait_idle(fork["id"])
        self.assertNotIn("<forked_conversation>", self.invocations()[-1]["prompt"])
        self.assertEqual(
            len(self.manager.view(pid, thread["id"])["turns"]),
            len(thread["turns"]),
        )
        with self.assertRaises(agents.AgentError):
            self.manager.fork(pid, thread["id"], turn_id="missing")

    def test_fork_transcript_keeps_requests_changes_and_final_replies(self):
        items = [
            {"turn": "a", "kind": "user", "text": "Make an intro"},
            {"turn": "a", "kind": "assistant", "text": "Let me look at the files."},
            {"turn": "a", "kind": "command", "detail": "x" * 5000},
            {
                "turn": "a",
                "kind": "file_change",
                "files": [{"path": "script.py"}, {"path": "narration.txt"}],
            },
            {"turn": "a", "kind": "file_change", "files": [{"path": "script.py"}]},
            {"turn": "a", "kind": "assistant", "text": "Created the intro."},
            {"turn": "b", "kind": "user", "text": "y" * 9000},
        ]
        text = agents.transcript(items)
        self.assertIn("User: Make an intro", text)
        self.assertIn("(Assistant changed: script.py, narration.txt)", text)
        self.assertIn("Assistant: Created the intro.", text)
        self.assertNotIn("Let me look", text)
        self.assertNotIn("xxxx", text)
        self.assertLess(len(text), 2600)  # Long messages keep their start and end.
        long = [
            {"turn": str(i), "kind": "user", "text": f"request {i} " + "z" * 1500}
            for i in range(30)
        ]
        text = agents.transcript(long)
        self.assertLess(len(text), agents.TRANSCRIPT_LIMIT + 300)
        self.assertIn("request 29", text)
        self.assertNotIn("request 0 ", text)

    def test_fork_can_stop_at_an_earlier_turn_and_switch_agent(self):
        thread = self.start("codex")
        pid = self.project["id"]
        first = thread["turns"][0]["id"]
        self.manager.send(pid, thread["id"], "Second request")
        self.wait_idle(thread["id"])
        fork = self.manager.fork(pid, thread["id"], turn_id=first, provider="claude")
        self.assertEqual(len(fork["turns"]), 1)
        self.assertEqual(fork["provider"], "claude")
        self.assertNotIn("Second request", [item.get("text") for item in fork["items"]])

    def test_interrupt_stops_the_process_tree(self):
        thread = self.manager.create(self.project["id"], provider="codex")
        self.manager.send(self.project["id"], thread["id"], "SLEEP")
        deadline = time.monotonic() + 15
        while not self.invocations() and time.monotonic() < deadline:
            time.sleep(0.05)
        with self.assertRaises(agents.AgentError):
            self.manager.send(self.project["id"], thread["id"], "second")
        other = self.manager.create(self.project["id"], provider="claude")
        with self.assertRaises(agents.AgentError):
            self.manager.send(self.project["id"], other["id"], "parallel edit")
        started = time.monotonic()
        self.manager.interrupt(self.project["id"], thread["id"], wait=True)
        self.assertLess(time.monotonic() - started, 10)
        view = self.manager.view(self.project["id"], thread["id"])
        self.assertEqual(view["status"], "interrupted")
        self.assertEqual(
            view["items"][-1],
            {**view["items"][-1], "kind": "notice", "text": "Stopped."},
        )
        open_rows = [
            item for item in view["items"] if item.get("status") == "in_progress"
        ]
        self.assertEqual(open_rows, [])
        self.assertFalse(
            (self.root / "projects" / "Demo" / "narration.txt").read_text().strip()
        )

    def test_unauthenticated_and_unknown_requests_are_rejected(self):
        thread = self.manager.create(self.project["id"], provider="codex")
        self.manager.statuses["codex"] = {"auth": {"status": "unauthenticated"}}
        with self.assertRaisesRegex(agents.AgentError, "Sign in"):
            self.manager.send(self.project["id"], thread["id"], "hi")
        with self.assertRaises(agents.AgentError):
            self.manager.send(self.project["id"], thread["id"], "   ")
        with self.assertRaises(KeyError):
            self.manager.view(self.project["id"], "f" * 32)
        with self.assertRaises(KeyError):
            self.manager.list_threads("../escape")
        with self.assertRaises(agents.AgentError):
            self.manager.create(self.project["id"], provider="gemini")
        self.manager.delete(self.project["id"], thread["id"])
        self.assertEqual(self.manager.list_threads(self.project["id"]), [])

    def test_threads_resume_after_restart_and_project_deletion_removes_them(self):
        thread = self.manager.create(self.project["id"], provider="claude")
        path = self.manager.thread_dir(self.project["id"]) / f"{thread['id']}.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        data["turns"].append({"id": "x", "status": "running", "finished": None})
        path.write_text(json.dumps(data), encoding="utf-8")
        reloaded = agents.AgentManager(
            self.root, python=sys.executable, app_root=self.root
        )
        self.assertEqual(
            reloaded.list_threads(self.project["id"])[0]["status"], "interrupted"
        )
        reloaded.delete_project(self.project["id"])
        self.assertFalse(path.parent.exists())


class EventBusTests(unittest.TestCase):
    def test_stream_delivers_published_events_and_closes(self):
        bus = EventBus()

        async def read():
            stream = bus.stream({"version": "1"})
            hello = await anext(stream)
            threading.Thread(
                target=lambda: bus.publish("render", {"render": {"id": "x"}})
            ).start()
            message = await asyncio.wait_for(anext(stream), 5)
            bus.close()
            rest = [chunk async for chunk in stream]
            return hello, message, rest

        hello, message, rest = asyncio.run(read())
        self.assertIn('"type": "hello"', hello)
        self.assertEqual(
            json.loads(message[6:]), {"type": "render", "render": {"id": "x"}}
        )
        self.assertEqual(rest, [])
        bus.open()
        self.assertFalse(bus.closed)


class AgentApiTests(FakeCliCase):
    def setUp(self):
        super().setUp()
        for name, value in (
            ("ROOT", self.root),
            ("AGENTS", self.manager),
            ("JOBS", {}),
            ("ACTIVE", None),
        ):
            item = patch.object(studio, name, value)
            item.start()
            self.addCleanup(item.stop)

    def test_thread_endpoints(self):
        pid = self.project["id"]
        created = studio.create_thread(pid, studio.ThreadCreate(provider="codex"))
        self.assertEqual(studio.list_threads(pid)["threads"][0]["id"], created["id"])
        renamed = studio.update_thread(
            pid, created["id"], studio.ThreadUpdate(title="Intro")
        )
        self.assertEqual(renamed["title"], "Intro")
        with self.assertRaises(studio.HTTPException) as missing:
            studio.read_thread(pid, "0" * 32)
        self.assertEqual(missing.exception.status_code, 404)
        with self.assertRaises(studio.HTTPException) as unknown_project:
            studio.create_thread("0" * 32, studio.ThreadCreate())
        self.assertEqual(unknown_project.exception.status_code, 404)
        studio.JOBS["job"] = {"project": pid}
        with (
            patch.object(studio, "ACTIVE", "job"),
            self.assertRaises(studio.HTTPException) as busy,
        ):
            studio.start_turn(pid, created["id"], studio.TurnRequest(text="go"))
        self.assertEqual(busy.exception.status_code, 409)
        studio.start_turn(pid, created["id"], studio.TurnRequest(text="go"))
        self.wait_idle(created["id"])
        self.assertEqual(studio.read_thread(pid, created["id"])["status"], "completed")
        studio.delete_thread(pid, created["id"])
        self.assertEqual(studio.list_threads(pid)["threads"], [])

    def test_project_deletion_waits_for_agents_and_cleans_threads(self):
        pid = self.project["id"]
        thread = studio.create_thread(pid, studio.ThreadCreate(provider="codex"))
        self.manager.send(pid, thread["id"], "SLEEP")
        with self.assertRaises(studio.HTTPException) as busy:
            studio.delete_project(pid)
        self.assertEqual(busy.exception.status_code, 409)
        self.manager.interrupt(pid, thread["id"], wait=True)
        studio.delete_project(pid)
        self.assertFalse(self.manager.thread_dir(pid).exists())

    def test_settings_endpoint_persists(self):
        result = studio.update_agent_settings(
            studio.AgentSettings(
                providers={"codex": {"model": "gpt-test"}}, auto_render=False
            )
        )
        self.assertEqual(result["providers"]["codex"]["model"], "gpt-test")
        saved = json.loads(
            (self.root / ".studio" / "agents.json").read_text(encoding="utf-8")
        )
        self.assertFalse(saved["auto_render"])
        self.manager.refresh(wait=True)

    def test_file_edits_wait_for_agent_but_renaming_is_allowed(self):
        pid = self.project["id"]
        thread = self.manager.create(pid, provider="codex")
        self.manager.send(pid, thread["id"], "SLEEP")
        with self.assertRaises(studio.HTTPException) as busy:
            studio.update_project(pid, studio.ProjectUpdate(source="new source"))
        self.assertEqual(busy.exception.status_code, 409)
        renamed = studio.update_project(pid, studio.ProjectUpdate(name="Renamed"))
        self.assertEqual(renamed["name"], "Renamed")
        self.manager.interrupt(pid, thread["id"], wait=True)


if __name__ == "__main__":
    unittest.main()
