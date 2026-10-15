"""Stands in for the codex and claude CLIs in agent tests.

FAKE_AGENT_LOG receives one JSON line per invocation: argv, cwd and stdin.
Prompts containing SLEEP wait to be interrupted; FAIL exits with an error.
"""

import json
import hashlib
import os
from pathlib import Path
import sys
import time
import re

argv = sys.argv[1:]


def emit(event):
    print(json.dumps(event), flush=True)


def log(prompt=""):
    path = os.environ.get("FAKE_AGENT_LOG")
    if path:
        references = []
        for encoded in re.findall(r'^"assets/.*"$', prompt, re.MULTILINE):
            name = json.loads(encoded)
            file = Path(name)
            if not file.resolve().is_relative_to(Path.cwd().resolve()):
                raise RuntimeError("Reference escaped the project")
            data = file.read_bytes()
            references.append(
                {
                    "path": name,
                    "size": len(data),
                    "sha256": hashlib.sha256(data).hexdigest(),
                }
            )
        with open(path, "a", encoding="utf-8") as output:
            output.write(
                json.dumps(
                    {
                        "argv": argv,
                        "cwd": os.getcwd(),
                        "prompt": prompt,
                        "references": references,
                    }
                )
                + "\n"
            )


def write_project():
    repairing = "Studio automatic video repair" in prompt
    if not repairing and "BROKEN_SYNTAX" in prompt:
        Path("script.py").write_text(
            "from manim import *\nclass Intro(Scene)\n    pass\n", encoding="utf-8"
        )
        return
    if not repairing and "BROKEN_RUNTIME" in prompt:
        Path("script.py").write_text(
            "from manim import *\nclass Intro(Scene):\n    def construct(self):\n        raise ValueError('fixture runtime error to repair')\n",
            encoding="utf-8",
        )
        return
    Path("script.py").write_text(
        "from manim import *\n\nclass Intro(Scene):\n    def construct(self):\n"
        "        self.play(Create(Circle()))\n        self.wait(2)\n",
        encoding="utf-8",
    )
    Path("narration.txt").write_text("[0:00] Here is a circle.\n", encoding="utf-8")
    Path("assets").mkdir(exist_ok=True)
    Path("assets/dot.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg"/>', encoding="utf-8"
    )


if argv == ["--version"]:
    print("fake-cli 1.2.3 (test)")
elif argv == ["login", "status"]:
    if os.environ.get("FAKE_AGENT_AUTH") == "none":
        print("Not logged in")
        sys.exit(1)
    print("Logged in using ChatGPT")
elif argv == ["auth", "status"]:
    signed_in = os.environ.get("FAKE_AGENT_AUTH") != "none"
    print(
        json.dumps(
            {
                "loggedIn": signed_in,
                "authMethod": "claude.ai",
                "email": "a@b.c",
                "subscriptionType": "pro",
            }
        )
    )
    sys.exit(0 if signed_in else 1)
elif argv == ["debug", "models"]:
    print(
        json.dumps(
            {
                "models": [
                    {
                        "slug": "gpt-test",
                        "display_name": "GPT-Test",
                        "visibility": "list",
                        "default_reasoning_level": "medium",
                        "supported_reasoning_levels": [
                            {"effort": "low"},
                            {"effort": "high"},
                        ],
                    },
                    {"slug": "hidden", "display_name": "Hidden", "visibility": "hide"},
                ]
            }
        )
    )
elif argv[:1] == ["exec"]:
    prompt = sys.stdin.read()
    log(prompt)
    session = argv[-2] if "resume" in argv else None  # ... resume <id> -
    emit(
        {
            "type": "thread.started",
            "thread_id": session or "11111111-2222-3333-4444-555555555555",
        }
    )
    emit({"type": "turn.started"})
    if "FAIL" in prompt:
        print("fatal: the model is unavailable", file=sys.stderr)
        sys.exit(2)
    emit(
        {
            "type": "item.completed",
            "item": {"id": "item_0", "type": "reasoning", "text": "Planning the scene"},
        }
    )
    command = '"C:\\\\pwsh.exe" -Command "python agent_check.py"'
    emit(
        {
            "type": "item.started",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": command,
                "aggregated_output": "",
                "exit_code": None,
                "status": "in_progress",
            },
        }
    )
    if "SLEEP" in prompt:
        time.sleep(60)
    write_project()
    emit(
        {
            "type": "item.completed",
            "item": {
                "id": "item_1",
                "type": "command_execution",
                "command": command,
                "aggregated_output": "OK: ready",
                "exit_code": 0,
                "status": "completed",
            },
        }
    )
    emit(
        {
            "type": "item.completed",
            "item": {
                "id": "item_2",
                "type": "file_change",
                "changes": [{"path": str(Path("script.py").resolve()), "kind": "add"}],
                "status": "completed",
            },
        }
    )
    emit(
        {
            "type": "item.completed",
            "item": {
                "id": "item_3",
                "type": "agent_message",
                "text": "Created an intro scene.",
            },
        }
    )
    emit({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 5}})
elif "-p" in argv:
    prompt = sys.stdin.read()
    log(prompt)
    session = (
        argv[argv.index("--resume") + 1]
        if "--resume" in argv
        else "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    )
    emit({"type": "system", "subtype": "init", "session_id": session})
    emit(
        {
            "type": "stream_event",
            "event": {"type": "message_start", "message": {"id": "msg_1"}},
        }
    )
    emit(
        {
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text"},
            },
        }
    )
    emit(
        {
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "Writing "},
            },
        }
    )
    emit(
        {
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": "the scene"},
            },
        }
    )
    emit(
        {
            "type": "assistant",
            "message": {
                "id": "msg_1",
                "content": [{"type": "text", "text": "Writing the scene."}],
            },
        }
    )
    if "SLEEP" in prompt:
        time.sleep(60)
    write_project()
    emit(
        {
            "type": "assistant",
            "message": {
                "id": "msg_1",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "tool_1",
                        "name": "Write",
                        "input": {
                            "file_path": str(Path("script.py").resolve()),
                            "content": "...",
                        },
                    }
                ],
            },
        }
    )
    emit(
        {
            "type": "assistant",
            "message": {
                "id": "msg_1",
                "content": [
                    {
                        "type": "tool_use",
                        "id": "tool_2",
                        "name": "Bash",
                        "input": {"command": "python agent_check.py"},
                    }
                ],
            },
        }
    )
    emit(
        {
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool_1",
                        "content": "File created",
                    }
                ]
            },
        }
    )
    emit(
        {
            "type": "user",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "tool_2",
                        "content": [{"type": "text", "text": "OK: ready"}],
                    }
                ]
            },
        }
    )
    emit(
        {
            "type": "assistant",
            "message": {"id": "msg_2", "content": [{"type": "text", "text": "Done."}]},
        }
    )
    if "FAIL" in prompt:
        emit(
            {
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": True,
                "result": "Ran out of turns",
            }
        )
    else:
        emit(
            {
                "type": "result",
                "subtype": "success",
                "is_error": False,
                "result": "Done.",
                "session_id": session,
            }
        )
else:
    print(f"unexpected arguments: {argv}", file=sys.stderr)
    sys.exit(64)
