"""Coding-agent CLIs (OpenAI Codex, Claude Code) that write Studio projects.

Each chat thread belongs to one project. A turn runs the provider's CLI in
non-interactive JSON streaming mode with the project folder as its working
directory, translates the stream into timeline items and publishes them.
Provider sessions are resumed on later turns, so the agent keeps context.
"""

from __future__ import annotations

from datetime import datetime, timezone
import json
import logging
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading
import time
import tomllib
import uuid

from events import publish
from processes import (
    attach_process_tree,
    popen_options,
    release_process_tree,
    stop_process,
    tree_active,
)
import projects

PROVIDERS = {
    "codex": {
        "name": "Codex",
        "command": "codex",
        "install": "npm install -g @openai/codex",
        "login": ["login"],
        "docs": "https://developers.openai.com/codex/cli",
    },
    "claude": {
        "name": "Claude Code",
        "command": "claude",
        "install": "irm https://claude.ai/install.ps1 | iex",
        "login": ["auth", "login"],
        "docs": "https://code.claude.com/docs/en/setup",
    },
}
CLAUDE_MODELS = [
    {
        "id": "claude-fable-5-1",
        "name": "Claude Fable 5.1",
        "description": "Most capable. Best for complex, multi-scene animations.",
        "efforts": ["low", "medium", "high", "xhigh", "max"],
        "default_effort": None,
    },
    {
        "id": "claude-opus-5-5",
        "name": "Claude Opus 5.5",
        "description": "Deep reasoning for demanding scripts.",
        "efforts": ["low", "medium", "high", "xhigh", "max"],
        "default_effort": None,
    },
    {
        "id": "claude-sonnet-5-5",
        "name": "Claude Sonnet 5.5",
        "description": "Balanced speed and quality. Recommended for most videos.",
        "efforts": ["low", "medium", "high", "xhigh", "max"],
        "default_effort": None,
    },
    {
        "id": "claude-haiku-5-5",
        "name": "Claude Haiku 5.5",
        "description": "Fastest and lightest. Good for quick edits.",
        "efforts": ["low", "medium", "high"],
        "default_effort": None,
    },
]
# Earlier releases stored these moving aliases; they keep working.
CLAUDE_ALIASES = {
    "opus": "Claude Opus (latest)",
    "sonnet": "Claude Sonnet (latest)",
    "haiku": "Claude Haiku (latest)",
    "fable": "Claude Fable (latest)",
}
CODEX_FALLBACK_MODELS = [
    {
        "id": "gpt-5.6-sol",
        "name": "GPT-5.6-Sol",
        "description": "OpenAI's coding model for Codex.",
        "efforts": ["low", "medium", "high", "xhigh"],
        "default_effort": "medium",
    },
]
EFFORTS = ("low", "medium", "high", "xhigh", "max", "ultra")
ACCESS_MODES = ("full", "edits")
MODEL_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:/\[\]-]{0,99}")
SESSION_PATTERN = re.compile(r"[A-Za-z0-9-]{8,80}")
ID_PATTERN = re.compile(r"[0-9a-f]{32}")
DEFAULT_SETTINGS = {
    "providers": {
        "codex": {
            "enabled": True,
            "binary": "",
            "model": "",
            "effort": "medium",
            "custom_models": [],
        },
        "claude": {
            "enabled": True,
            "binary": "",
            "model": "claude-sonnet-5-5",
            "effort": "",
            "custom_models": [],
        },
    },
    "default_provider": "",
    "access": "full",
    "auto_render": True,
    # How the render queue starts ("auto" or "manual") and what it does when a
    # video fails ("fix" with the agent and re-queue, "skip" it, or "pause").
    "queue_start": "auto",
    "queue_on_error": "fix",
    # Keep the installed CLIs current (see agent_updates.py).
    "auto_update_agents": True,
}
QUEUE_START_MODES = ("auto", "manual")
QUEUE_ERROR_MODES = ("fix", "skip", "pause")
PROBE_TIMEOUT = 20
MAX_PROMPT = 40_000
MAX_DETAIL = 6_000
MAX_ITEMS = 3_000
MAX_STDERR = 20_000
# How long a new turn waits for its provider's CLI update to finish.
UPDATE_WAIT = 30


ERROR_CODES = [
    (
        "usage_limit",
        "Usage limit reached",
        "Your plan's usage allowance is used up. Wait for it to reset, or switch to another model or agent.",
        re.compile(
            r"usage limit|hit your (?:usage |weekly |5-hour |session )?limit|limit reached"
            r"|out of (?:credits|usage|messages)|(?:5-hour|weekly|session|daily) limit"
            r"|exceeded your (?:current )?(?:usage|plan|quota)",
            re.I,
        ),
    ),
    (
        "quota_exceeded",
        "Billing or quota problem",
        "The account behind this agent has no remaining credit or quota. Check its billing settings.",
        re.compile(
            r"insufficient[_ ]quota|credit balance|billing|payment required|\b402\b",
            re.I,
        ),
    ),
    (
        "rate_limited",
        "Rate limited",
        "The provider is receiving too many requests. Wait a minute and send the message again.",
        re.compile(r"rate[ _-]?limit|too many requests|\b429\b", re.I),
    ),
    (
        "context_too_long",
        "Conversation too long",
        "This chat has outgrown the model's context window. Start a new chat or fork from an earlier point.",
        re.compile(
            r"context (?:length|window)|prompt is too long|maximum context|too many tokens"
            r"|token limit|context_length_exceeded",
            re.I,
        ),
    ),
    (
        "model_unavailable",
        "Model not available",
        "This model isn't available to your account. Choose another model from the picker.",
        re.compile(
            r"model[^.\n]{0,60}(?:not found|not available|unavailable|does not exist|unsupported|"
            r"unknown|no access|not supported)|invalid model|unknown model",
            re.I,
        ),
    ),
    (
        "auth_required",
        "Sign-in required",
        "The agent isn't signed in or its session expired. Sign in again in Settings → Providers.",
        re.compile(
            r"not logged in|please (?:log|sign) ?in|unauthori[sz]ed|invalid api key"
            r"|authentication|\b401\b|expired (?:token|session)|/login",
            re.I,
        ),
    ),
    (
        "overloaded",
        "Provider is overloaded",
        "The provider is temporarily at capacity. Try again in a few minutes.",
        re.compile(
            r"overloaded|\b529\b|\b503\b|service unavailable|at capacity|try again later",
            re.I,
        ),
    ),
    (
        "network_error",
        "Connection problem",
        "Studio couldn't reach the provider. Check your internet connection and try again.",
        re.compile(
            r"ENOTFOUND|ECONNRESET|ECONNREFUSED|ETIMEDOUT|network|connection (?:error|refused|reset|closed)"
            r"|fetch failed|timed out|could not resolve|dns",
            re.I,
        ),
    ),
]
RESET_HINT = re.compile(
    r"((?:resets?|try again|available again|retry)[^.\n|]{0,80})", re.I
)
HTTP_STATUS = re.compile(r"\b([45]\d\d)\b")


def classify_error(message: str, status=None) -> dict:
    """Turn a provider failure into a stable code, headline and next step."""
    text = str(message or "")
    probe = f"{status} {text}" if status else text
    for code, title, hint, pattern in ERROR_CODES:
        if pattern.search(probe):
            break
    else:
        code, title, hint = (
            "agent_error",
            "The agent stopped with an error",
            "Open the details below, then retry or start a new chat.",
        )
    info = {"code": code, "title": title, "hint": hint}
    if isinstance(status, int):
        info["http_status"] = status
    else:
        found = HTTP_STATUS.search(text)
        if found and code != "agent_error":
            info["http_status"] = int(found[1])
    reset = RESET_HINT.search(text)
    if reset and code in {
        "usage_limit",
        "rate_limited",
        "overloaded",
        "quota_exceeded",
    }:
        info["reset"] = reset[1].strip(" .,:;")[:120]
    return info


class AgentError(Exception):
    """A user-facing agent problem."""


def _now():
    return datetime.now(timezone.utc).isoformat()


def _write_json(path: Path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.{uuid.uuid4().hex[:8]}.tmp")
    temporary.write_text(
        json.dumps(value, ensure_ascii=False, indent=1), encoding="utf-8"
    )
    temporary.replace(path)


def _clip(text, limit=MAX_DETAIL):
    text = str(text or "")
    return text if len(text) <= limit else "…" + text[-limit:]


# ---------------------------------------------------------------- settings


def clean_settings(value) -> dict:
    result = json.loads(json.dumps(DEFAULT_SETTINGS))
    if not isinstance(value, dict):
        return result
    for provider, defaults in result["providers"].items():
        saved = (value.get("providers") or {}).get(provider)
        if not isinstance(saved, dict):
            continue
        if isinstance(saved.get("enabled"), bool):
            defaults["enabled"] = saved["enabled"]
        binary = saved.get("binary")
        if isinstance(binary, str) and len(binary) <= 500 and "\n" not in binary:
            defaults["binary"] = binary.strip().strip('"')
        model = saved.get("model")
        if isinstance(model, str) and (model == "" or MODEL_PATTERN.fullmatch(model)):
            defaults["model"] = model
        effort = saved.get("effort")
        if effort == "" or effort in EFFORTS:
            defaults["effort"] = effort
        custom = saved.get("custom_models")
        if isinstance(custom, list):
            defaults["custom_models"] = [
                item
                for item in custom[:20]
                if isinstance(item, str) and MODEL_PATTERN.fullmatch(item)
            ]
    if value.get("default_provider") in ("", *PROVIDERS):
        result["default_provider"] = value["default_provider"]
    if value.get("access") in ACCESS_MODES:
        result["access"] = value["access"]
    if isinstance(value.get("auto_render"), bool):
        result["auto_render"] = value["auto_render"]
    if value.get("queue_start") in QUEUE_START_MODES:
        result["queue_start"] = value["queue_start"]
    if value.get("queue_on_error") in QUEUE_ERROR_MODES:
        result["queue_on_error"] = value["queue_on_error"]
    if isinstance(value.get("auto_update_agents"), bool):
        result["auto_update_agents"] = value["auto_update_agents"]
    return result


# ---------------------------------------------------------------- binaries


def _known_locations(provider: str) -> list[Path]:
    home = Path.home()
    local = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
    roaming = Path(os.environ.get("APPDATA", home / "AppData" / "Roaming"))
    if provider == "codex":
        return [
            local / "Programs" / "OpenAI" / "Codex" / "bin" / "codex.exe",
            roaming / "npm" / "codex.cmd",
            home / ".local" / "bin" / "codex",
            Path("/usr/local/bin/codex"),
            Path("/opt/homebrew/bin/codex"),
        ]
    return [
        home / ".local" / "bin" / "claude.exe",
        home / ".local" / "bin" / "claude",
        roaming / "npm" / "claude.cmd",
        home / ".claude" / "local" / "claude",
        Path("/usr/local/bin/claude"),
        Path("/opt/homebrew/bin/claude"),
    ]


def resolve_binary(provider: str, configured: str = "") -> str | None:
    if configured:
        path = Path(os.path.expandvars(configured)).expanduser()
        if path.is_file():
            return str(path)
        found = shutil.which(configured)
        return found
    found = shutil.which(PROVIDERS[provider]["command"])
    if found:
        return found
    return next(
        (str(path) for path in _known_locations(provider) if path.is_file()), None
    )


def launch_command(binary: str) -> list[str]:
    """The argv prefix that starts a provider binary."""
    return [binary]


def _run(command: list[str], timeout=PROBE_TIMEOUT) -> subprocess.CompletedProcess:
    return subprocess.run(
        [*launch_command(command[0]), *command[1:]],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        stdin=subprocess.DEVNULL,
        **({"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}),
    )


def _version(output: str) -> str | None:
    match = re.search(r"\d+\.\d+\.\d+(?:[-+.][0-9A-Za-z.]+)?", output)
    return match[0] if match else None


# ---------------------------------------------------------------- lean sessions
#
# Studio sessions only need file and shell tools. Everything else a CLI loads by
# default (the user's MCP servers, plugins, skills, hooks, memory files, web and
# sub-agent tools) is resent with every model request and counts against the
# user's plan. CLI flags change between releases, so each option is used only
# when the installed binary advertises (or accepts) it; an unknown flag must never
# break a run. Results are cached per binary file version.

CLAUDE_TOOLS = (
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "Glob",
    "Grep",
    "Bash",
    "PowerShell",
)
CLAUDE_ENV = {
    # Personal CLAUDE.md files and auto-memory are not about Studio projects.
    "CLAUDE_CODE_DISABLE_CLAUDE_MDS": "1",
    "CLAUDE_CODE_DISABLE_AUTO_MEMORY": "1",
}
# Codex features that only add tools or context an animation agent never uses.
CODEX_DISABLED_FEATURES = (
    "apps",
    "plugins",
    "remote_plugin",
    "recommended_plugins",
    "multi_agent",
    "multi_agent_v2",
    "image_generation",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "in_app_browser",
    "tool_suggest",
    "skill_mcp_dependency_install",
    "goals",
    "memories",
    "sleep_tool",
    "web_search_request",
    "web_search_cached",
)
CODEX_OVERRIDES = ('web_search="disabled"', "project_doc_max_bytes=0")
_CAPABILITIES: dict[tuple, object] = {}
_CAPABILITY_LOCK = threading.Lock()


def _binary_stamp(binary: str) -> tuple:
    try:
        stat = Path(binary).stat()
        return (str(binary), stat.st_mtime_ns, stat.st_size)
    except OSError:
        return (str(binary), None, None)


def _cached(key: tuple, compute):
    with _CAPABILITY_LOCK:
        if key in _CAPABILITIES:
            return _CAPABILITIES[key]
    value = compute()
    if value is not None:  # A failed probe is retried on the next turn.
        with _CAPABILITY_LOCK:
            _CAPABILITIES[key] = value
    return value


def _accepts(binary: str, *args: str) -> str | None:
    """Output of a local, offline CLI command, or None when it fails."""
    try:
        result = _run([binary, *args])
    except (OSError, subprocess.SubprocessError):
        return None
    if result.returncode:
        return None
    return (result.stdout or "") + (result.stderr or "")


def cli_flags(binary: str, *subcommand: str) -> frozenset[str]:
    """Long options listed by `<binary> [subcommand] --help`."""

    def compute():
        text = _accepts(binary, *subcommand, "--help")
        if text is None:
            return None
        return frozenset(re.findall(r"(?<![\w-])--[a-z][a-z0-9-]*", text))

    return _cached(("flags", _binary_stamp(binary), subcommand), compute) or frozenset()


def _codex_features(binary: str) -> frozenset[str]:
    def compute():
        text = _accepts(binary, "features", "list")
        if text is None:
            return None
        names = set()
        for line in text.splitlines():
            parts = line.split()
            if len(parts) >= 2 and "removed" not in line:
                names.add(parts[0])
        return frozenset(names)

    return _cached(("features", _binary_stamp(binary)), compute) or frozenset()


def _codex_mcp_servers() -> list[str]:
    """MCP servers in the user's Codex config, so a session can switch them off."""
    home = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex")
    try:
        with (home / "config.toml").open("rb") as handle:
            servers = tomllib.load(handle).get("mcp_servers") or {}
    except (OSError, ValueError):
        return []
    if not isinstance(servers, dict):
        return []
    return sorted(
        name
        for name, config in servers.items()
        if isinstance(config, dict) and re.fullmatch(r"[A-Za-z0-9_-]+", name)
    )


def codex_overrides(binary: str) -> list[str]:
    """`-c` overrides that keep a Codex session lean, checked against the binary.

    Codex rejects a known key with the wrong type, so the set is first loaded by
    an offline command; anything this version refuses is left out.
    """
    features = _codex_features(binary)
    wanted = list(CODEX_OVERRIDES)
    wanted += [
        f"features.{name}=false" for name in CODEX_DISABLED_FEATURES if name in features
    ]
    wanted += [f"mcp_servers.{name}.enabled=false" for name in _codex_mcp_servers()]

    def loads(overrides) -> bool:
        args = [part for value in overrides for part in ("-c", value)]
        return _accepts(binary, *args, "features", "list") is not None

    def compute():
        if not features:
            return None  # This Codex can't verify settings offline; send none.
        if loads(wanted):
            return tuple(wanted)
        return tuple(value for value in wanted if loads([value]))

    return list(_cached(("codex", _binary_stamp(binary), tuple(wanted)), compute) or ())


def claude_options(binary: str, prompt_dir: Path) -> list[str]:
    """Flags that keep a Claude Code session to Studio's own tools and context."""
    flags = cli_flags(binary)
    options = []
    if {"--strict-mcp-config", "--mcp-config"} <= flags:
        config = prompt_dir / "mcp.json"
        config.write_text('{"mcpServers": {}}', encoding="utf-8")
        options += ["--strict-mcp-config", "--mcp-config", str(config)]
    if "--setting-sources" in flags:
        # User settings may carry sign-in helpers or provider variables; project
        # and local settings would only come from folders above the project.
        options += ["--setting-sources", "user"]
    if "--settings" in flags:
        settings = prompt_dir / "settings.json"
        settings.write_text('{"disableAllHooks": true}', encoding="utf-8")
        options += ["--settings", str(settings)]
    if "--disable-slash-commands" in flags:
        options.append("--disable-slash-commands")
    if "--tools" in flags:
        options += ["--tools", ",".join(CLAUDE_TOOLS)]
    return options


def probe(provider: str, settings: dict) -> dict:
    """Health check: installed binary, version, authentication and models."""
    meta = PROVIDERS[provider]
    config = settings["providers"][provider]
    status = {
        "id": provider,
        "name": meta["name"],
        "enabled": config["enabled"],
        "installed": False,
        "binary": None,
        "version": None,
        "auth": {"status": "unknown"},
        "status": "error",
        "message": "",
        "models": [],
        "install": meta["install"],
        "docs": meta["docs"],
        "checked": _now(),
    }
    status["models"] = _models(provider, None, config)
    if not config["enabled"]:
        status.update(status="disabled", message=f"{meta['name']} is turned off.")
        return status
    binary = resolve_binary(provider, config["binary"])
    if not binary:
        status["message"] = (
            f"The configured {meta['name']} path was not found."
            if config["binary"]
            else f"{meta['name']} (`{meta['command']}`) was not found. Install it, then refresh."
        )
        return status
    status.update(installed=True, binary=binary)
    try:
        result = _run([binary, "--version"])
    except (OSError, subprocess.SubprocessError) as error:
        status["message"] = f"{meta['name']} is installed but could not start: {error}"
        return status
    status["version"] = _version(result.stdout + "\n" + result.stderr)
    if result.returncode:
        status["message"] = f"{meta['name']} is installed but failed to run."
        return status
    try:
        status["auth"] = _auth(provider, binary)
    except (OSError, subprocess.SubprocessError, ValueError):
        status["auth"] = {"status": "unknown"}
    status["models"] = _models(provider, binary, config)
    if status["auth"]["status"] == "unauthenticated":
        status.update(status="warning", message=f"Sign in to {meta['name']} to use it.")
    else:
        status.update(status="ready", message="")
    return status


def _auth(provider: str, binary: str) -> dict:
    if provider == "claude":
        result = _run([binary, "auth", "status"])
        try:
            data = json.loads(result.stdout)
        except ValueError:
            data = None
        if isinstance(data, dict) and isinstance(data.get("loggedIn"), bool):
            if not data["loggedIn"]:
                return {"status": "unauthenticated"}
            plan = data.get("subscriptionType")
            method = data.get("authMethod")
            return {
                "status": "authenticated",
                "label": f"Claude {plan.title()}"
                if isinstance(plan, str)
                else method or "Signed in",
                "account": data.get("email")
                if isinstance(data.get("email"), str)
                else None,
            }
        return {"status": "unauthenticated" if result.returncode else "unknown"}
    result = _run([binary, "login", "status"])
    text = (result.stdout + "\n" + result.stderr).strip()
    if re.search(r"not logged in", text, re.I):
        return {"status": "unauthenticated"}
    match = re.search(r"logged in using (.+)", text, re.I)
    if result.returncode == 0 and match:
        return {"status": "authenticated", "label": match[1].strip().rstrip(".")}
    return {"status": "unauthenticated" if result.returncode else "unknown"}


def _models(provider: str, binary: str | None, config: dict) -> list[dict]:
    models = []
    if provider == "claude":
        models = [dict(model) for model in CLAUDE_MODELS]
    elif binary:
        try:
            result = _run([binary, "debug", "models"], timeout=30)
            catalog = json.loads(result.stdout)
            for model in catalog.get("models", []):
                if not isinstance(model, dict) or model.get("visibility") != "list":
                    continue
                slug = model.get("slug")
                if not isinstance(slug, str) or not MODEL_PATTERN.fullmatch(slug):
                    continue
                efforts = [
                    level.get("effort")
                    for level in model.get("supported_reasoning_levels", [])
                    if isinstance(level, dict) and level.get("effort") in EFFORTS
                ]
                models.append(
                    {
                        "id": slug,
                        "name": str(model.get("display_name") or slug)[:60],
                        "description": str(model.get("description") or "")[:200],
                        "efforts": efforts,
                        "default_effort": model.get("default_reasoning_level")
                        if model.get("default_reasoning_level") in EFFORTS
                        else None,
                    }
                )
        except (OSError, subprocess.SubprocessError, ValueError, AttributeError):
            models = []
    if not models and provider == "codex":
        models = [dict(model) for model in CODEX_FALLBACK_MODELS]
    if provider == "claude":
        for alias, label in CLAUDE_ALIASES.items():
            models.append(
                {
                    "id": alias,
                    "name": label,
                    "description": "Moving alias that follows the newest release.",
                    "efforts": list(EFFORTS[:5]),
                    "default_effort": None,
                    "alias": True,
                }
            )
    known = {model["id"] for model in models}
    for custom in config["custom_models"]:
        if custom not in known:
            models.append(
                {
                    "id": custom,
                    "name": custom,
                    "efforts": list(EFFORTS[:4]),
                    "custom": True,
                }
            )
    return models


# ---------------------------------------------------------------- instructions


def instructions(project_dir: Path, *, python: str, check: str, latex: bool) -> str:
    equations = (
        "MathTex/Tex (LaTeX is installed) or MathTypst/Typst"
        if latex
        else "MathTypst/Typst (LaTeX is not installed, so never use Tex or MathTex)"
    )
    return f"""You are the animation agent inside Manim Studio, a desktop app that turns
Manim Community Edition scripts into narrated explainer videos. Your working
directory is one Studio project folder:
{project_dir}

Project files
- script.py: the scene Studio renders. Keep one main Scene subclass.
- narration.txt: the voice-over, spoken by a local neural voice and mixed into the video.
- assets/: user references, images (PNG/JPG), SVGs and sounds. Read attached
  references before writing. Use relative paths such as "assets/diagram.svg".
- project.json: Studio metadata; never edit it. Ignore hidden folders.

Manim rules
- Manim CE 0.21, Cairo renderer, Python 3.12, `from manim import *`. Never use
  manimlib/ManimGL or OpenGL-only features.
- Equations: {equations}. Use Text or MarkupText for prose.
- Only the standard library, manim and its dependencies (numpy, scipy, Pillow,
  networkx). Never install packages, download files or use the network.
- The frame is about 14.2 x 8 units. Keep everything on screen and readable: use
  arrange, next_to and buff to avoid overlaps, scale long text to fit, fade out old
  content before new sections, keep one consistent color palette.
- Give each idea time with run_time and self.wait() so the video carries the
  narration. End with a short self.wait().

Narration (narration.txt)
- One paragraph per spoken line, separated by blank lines. Lines starting with #
  are notes, not spoken.
- A paragraph may start with a timestamp such as [0:05] to pin its start; match
  it to the animation (sum of run_time and wait before that moment). Otherwise a
  line follows the previous one.
- Write for the ear: short sentences, everyday words, contractions, commas where a
  speaker pauses, varied sentence length, the occasional rhetorical question.
  Spell out math and symbols ("x squared", "the integral from zero to one").
  No markdown, emoji, abbreviations, URLs, brackets or parenthetical asides.
- Speech runs about 2.3 words per second; make the animation long enough for
  every line.

Assets
- Write diagrams, icons and illustrations as clean SVG files in assets/, or make
  raster images with a short Pillow/numpy helper run with "{python}", then delete
  the helper.
- Use ImageMobject("assets/name.png"), SVGMobject("assets/name.svg") and
  self.add_sound("assets/name.wav").

Working efficiently
- Read only the files the task needs. The user may edit files between messages,
  but within one task you know a file after writing or editing it, so don't read
  it again. Prefer small edits to whole-file rewrites.
- Rely on your Manim knowledge; open library source only when an API is unclear.

Checking your work
- When your edits are done, run this check from the project folder (it executes
  the scene without writing video), fix what it reports, and re-run it only after
  a fix: {check}
- Do not render videos yourself; Studio renders automatically when you finish.

Finish with a brief summary of what you created or changed, as a few short
bullet points. Do not paste the whole script into your reply."""


# ---------------------------------------------------------------- stream adapters


def _relative(path: str, root: Path) -> str:
    try:
        return Path(path).resolve().relative_to(root.resolve()).as_posix()
    except (ValueError, OSError, RuntimeError):
        return str(path)


_SHELL_WRAPPER = re.compile(
    r"""^"?[^"]*?(?:pwsh|powershell|cmd|bash|sh)(?:\.exe)?"?\s+(?:-NoProfile\s+)?(?:-Command|-c|/c)\s+(.*)$""",
    re.I | re.S,
)


def display_command(command) -> str:
    if isinstance(command, list):
        command = " ".join(str(part) for part in command)
    command = str(command or "").strip()
    match = _SHELL_WRAPPER.match(command)
    if match:
        command = match[1].strip()
        if len(command) > 1 and command[0] == command[-1] and command[0] in "'\"":
            command = command[1:-1]
    return command[:2000]


class CodexStream:
    """Translate `codex exec --json` events."""

    def __init__(self, turn: "Turn"):
        self.turn = turn

    def handle(self, event: dict):
        kind = event.get("type")
        if kind == "thread.started" and isinstance(event.get("thread_id"), str):
            self.turn.set_session(event["thread_id"])
        elif kind in ("item.started", "item.updated", "item.completed"):
            item = event.get("item")
            if isinstance(item, dict):
                self._item(item, kind == "item.completed")
        elif kind == "turn.completed":
            self.turn.usage = (
                event.get("usage") if isinstance(event.get("usage"), dict) else None
            )
        elif kind == "turn.failed":
            error = event.get("error") or {}
            self.turn.fail(str(error.get("message") or "The turn failed."))
        elif kind == "error":
            message = str(event.get("message") or "Codex reported an error.")
            # Reconnection notices are transient; keep only the final failure.
            if not re.search(r"reconnecting", message, re.I):
                self.turn.fail(message)

    def _item(self, item: dict, done: bool):
        key = f"codex:{item.get('id')}"
        kind = item.get("type")
        root = self.turn.project_dir
        status = item.get("status") or ("completed" if done else "in_progress")
        if kind == "agent_message":
            self.turn.upsert(
                key,
                kind="assistant",
                text=str(item.get("text") or ""),
                streaming=not done,
            )
        elif kind == "reasoning":
            text = str(item.get("text") or "").strip()
            if text:
                self.turn.upsert(key, kind="reasoning", text=_clip(text))
        elif kind == "command_execution":
            exit_code = item.get("exit_code")
            self.turn.upsert(
                key,
                kind="command",
                title=display_command(item.get("command")),
                detail=_clip(item.get("aggregated_output")),
                status="failed"
                if status == "failed" or (done and exit_code not in (0, None))
                else status,
            )
        elif kind == "file_change":
            changes = [
                {
                    "path": _relative(change.get("path", ""), root),
                    "kind": change.get("kind", "update"),
                }
                for change in item.get("changes", [])
                if isinstance(change, dict)
            ]
            self.turn.upsert(key, kind="file_change", files=changes, status=status)
        elif kind == "mcp_tool_call":
            title = f"{item.get('server', 'tool')}.{item.get('tool', '')}".strip(".")
            self.turn.upsert(key, kind="tool", title=title, status=status)
        elif kind == "web_search":
            self.turn.upsert(
                key,
                kind="tool",
                title=f"Searched the web: {item.get('query', '')}",
                status=status,
            )
        elif kind == "todo_list":
            entries = [
                {
                    "text": str(entry.get("text", ""))[:300],
                    "done": bool(entry.get("completed")),
                }
                for entry in item.get("items", [])
                if isinstance(entry, dict)
            ]
            self.turn.upsert(key, kind="plan", entries=entries)
        elif kind == "error":
            self.turn.upsert(
                key, kind="error", text=str(item.get("message") or "Error")
            )


class ClaudeStream:
    """Translate `claude -p --output-format stream-json` events."""

    FILE_TOOLS = {"Write", "Edit", "MultiEdit", "NotebookEdit"}
    SHELL_TOOLS = {"Bash", "PowerShell"}

    def __init__(self, turn: "Turn"):
        self.turn = turn
        self.message = None
        self.blocks: dict[int, str] = {}
        self.ordinals: dict[str, int] = {}
        self.tools: dict[str, str] = {}

    def handle(self, event: dict):
        kind = event.get("type")
        if event.get("parent_tool_use_id"):
            return  # Sub-agent chatter stays inside its Task tool row.
        if kind == "system" and event.get("subtype") == "init":
            if isinstance(event.get("session_id"), str):
                self.turn.set_session(event["session_id"])
        elif kind == "stream_event":
            self._partial(event.get("event") or {})
        elif kind == "assistant":
            self._assistant(event.get("message") or {})
        elif kind == "user":
            self._results(event.get("message") or {})
        elif kind == "result":
            self.turn.usage = {
                key: event.get(key)
                for key in ("usage", "total_cost_usd", "num_turns", "duration_ms")
            }
            if event.get("is_error") or event.get("subtype") not in (None, "success"):
                status = event.get("api_error_status")
                message = str(
                    event.get("result")
                    or event.get("subtype")
                    or "Claude reported an error."
                )
                if isinstance(status, int):
                    self.turn.fail(message, status)
                else:
                    self.turn.fail(message)
        elif kind == "rate_limit_event":
            info = event.get("rate_limit_info")
            if isinstance(info, dict) and info.get("status") == "rejected":
                resets = info.get("resetsAt")
                kind_name = str(info.get("rateLimitType") or "usage").replace("_", " ")
                label = f"{kind_name} limit"
                if isinstance(resets, (int, float)) and resets > 0:
                    moment = datetime.fromtimestamp(resets).strftime("%b %d, %H:%M")
                    label += f" resets {moment}"
                self.turn.limit = label

    def _text_key(self, message_id: str, ordinal: int) -> str:
        return f"claude:{message_id}:text:{ordinal}"

    def _partial(self, event: dict):
        kind = event.get("type")
        if kind == "message_start":
            self.message = (event.get("message") or {}).get("id")
            self.blocks = {}
            self.ordinals.setdefault(self.message or "", 0)
        elif kind == "content_block_start" and self.message:
            block = event.get("content_block") or {}
            if block.get("type") == "text":
                ordinal = sum(
                    1 for value in self.blocks.values() if value.startswith("text")
                )
                self.blocks[event.get("index", 0)] = f"text:{ordinal}"
                self.turn.upsert(
                    self._text_key(self.message, ordinal),
                    kind="assistant",
                    text="",
                    streaming=True,
                )
        elif kind == "content_block_delta" and self.message:
            delta = event.get("delta") or {}
            block = self.blocks.get(event.get("index", 0), "")
            if delta.get("type") == "text_delta" and block.startswith("text"):
                key = self._text_key(self.message, int(block.split(":")[1]))
                self.turn.append_text(key, str(delta.get("text") or ""))

    def _assistant(self, message: dict):
        message_id = str(message.get("id") or uuid.uuid4().hex)
        for block in message.get("content") or []:
            if not isinstance(block, dict):
                continue
            kind = block.get("type")
            if kind == "text":
                ordinal = self.ordinals.get(message_id, 0)
                self.ordinals[message_id] = ordinal + 1
                self.turn.upsert(
                    self._text_key(message_id, ordinal),
                    kind="assistant",
                    text=str(block.get("text") or ""),
                    streaming=False,
                )
            elif kind == "thinking" and str(block.get("thinking") or "").strip():
                self.turn.upsert(
                    f"claude:{message_id}:thinking",
                    kind="reasoning",
                    text=_clip(block["thinking"]),
                )
            elif kind == "tool_use":
                self._tool(block)

    def _tool(self, block: dict):
        tool_id = str(block.get("id") or uuid.uuid4().hex)
        name = str(block.get("name") or "Tool")
        data = block.get("input") if isinstance(block.get("input"), dict) else {}
        key = f"claude:tool:{tool_id}"
        self.tools[tool_id] = key
        root = self.turn.project_dir
        if name in self.FILE_TOOLS:
            path = data.get("file_path") or data.get("notebook_path") or ""
            kind = "add" if name == "Write" else "update"
            self.turn.upsert(
                key,
                kind="file_change",
                files=[{"path": _relative(path, root), "kind": kind}],
                status="in_progress",
            )
        elif name in self.SHELL_TOOLS:
            self.turn.upsert(
                key,
                kind="command",
                title=display_command(data.get("command")),
                detail="",
                status="in_progress",
            )
        elif name == "TodoWrite":
            entries = [
                {
                    "text": str(todo.get("content", ""))[:300],
                    "done": todo.get("status") == "completed",
                }
                for todo in data.get("todos", [])
                if isinstance(todo, dict)
            ]
            self.turn.upsert("claude:plan", kind="plan", entries=entries)
        else:
            target = (
                data.get("file_path")
                or data.get("pattern")
                or data.get("path")
                or data.get("description")
                or ""
            )
            title = f"{name} {_relative(target, root) if target else ''}".strip()
            self.turn.upsert(key, kind="tool", title=title[:300], status="in_progress")

    def _results(self, message: dict):
        content = message.get("content")
        if not isinstance(content, list):
            return
        for block in content:
            if not isinstance(block, dict) or block.get("type") != "tool_result":
                continue
            key = self.tools.get(str(block.get("tool_use_id")))
            if not key:
                continue
            output = block.get("content")
            if isinstance(output, list):
                output = "\n".join(
                    str(part.get("text", ""))
                    for part in output
                    if isinstance(part, dict)
                )
            fields = {"status": "failed" if block.get("is_error") else "completed"}
            if self.turn.items_by_key.get(key, {}).get("kind") == "command":
                fields["detail"] = _clip(output)
            elif block.get("is_error"):
                fields["detail"] = _clip(output, 1000)
            self.turn.upsert(key, **fields)


# ---------------------------------------------------------------- threads


class Turn:
    """One running CLI invocation and the timeline items it produces."""

    def __init__(
        self, manager: "AgentManager", thread: dict, project_dir: Path, turn: dict
    ):
        self.manager = manager
        self.thread = thread
        self.project_dir = project_dir
        self.record = turn
        self.items_by_key: dict[str, dict] = {}
        self.usage = None
        self.error = None
        self.error_info = None
        self.limit = None
        self.process: subprocess.Popen | None = None
        self.cancelled = False

    def set_session(self, session_id: str):
        if SESSION_PATTERN.fullmatch(session_id):
            with self.manager.lock:
                self.thread["session"] = {
                    "provider": self.record["provider"],
                    "id": session_id,
                }
            self.manager.save(self.thread)

    def upsert(self, key: str, **fields):
        with self.manager.lock:
            item = self.items_by_key.get(key)
            if item is None:
                item = {
                    "id": uuid.uuid4().hex,
                    "turn": self.record["id"],
                    "created": _now(),
                }
                self.items_by_key[key] = item
                self.thread["items"].append(item)
                del self.thread["items"][:-MAX_ITEMS]
            item.update(fields)
            snapshot = dict(item)
        self.manager.emit_item(self.thread, snapshot)

    def append_text(self, key: str, text: str):
        with self.manager.lock:
            item = self.items_by_key.get(key)
            if item is None:
                return
            item["text"] = item.get("text", "") + text
            snapshot = dict(item)
        self.manager.emit_item(self.thread, snapshot, save=False)

    def fail(self, message: str, status=None):
        self.error = message[:4000]
        self.error_info = classify_error(message, status)
        # A structured rate-limit event knows exactly when the limit resets.
        if self.limit and self.error_info["code"] in {"usage_limit", "rate_limited"}:
            self.error_info["reset"] = self.limit


TRANSCRIPT_LIMIT = 12_000
TRANSCRIPT_MESSAGE_LIMIT = 2_000


def token_usage(usage) -> dict | None:
    """Input (cached included), cached and output tokens from either CLI's usage."""
    if not isinstance(usage, dict):
        return None
    if isinstance(usage.get("usage"), dict):  # Claude's result event
        usage = usage["usage"]

    def count(*keys):
        return sum(
            value
            for value in (usage.get(key) for key in keys)
            if isinstance(value, int) and not isinstance(value, bool) and value > 0
        )

    cached = count("cache_read_input_tokens", "cached_input_tokens")
    tokens = {
        # Codex counts cached input inside input_tokens; Claude reports it apart.
        "input": count(
            "input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"
        ),
        "cached": cached,
        "output": count("output_tokens"),
    }
    return tokens if tokens["input"] or tokens["output"] else None


def _shorten(text: str, limit=TRANSCRIPT_MESSAGE_LIMIT) -> str:
    text = text.strip()
    if len(text) <= limit:
        return text
    half = limit // 2
    return text[:half].rstrip() + "\n…\n" + text[-half:].lstrip()


def transcript(items: list[dict]) -> str:
    """A compact replay of a chat: each request, the files the agent changed and
    its final reply. Progress notes and tool output are left out because the
    project folder already holds the result; the newest turns win when long."""
    turns: dict[str, dict] = {}
    for item in items:
        turn = turns.setdefault(
            str(item.get("turn")), {"user": [], "files": [], "reply": ""}
        )
        kind = item.get("kind")
        if kind == "user" and str(item.get("text") or "").strip():
            turn["user"].append(item["text"])
        elif kind == "assistant" and str(item.get("text") or "").strip():
            turn["reply"] = item["text"]
        elif kind == "file_change":
            for change in item.get("files", []):
                path = change.get("path", "")
                if path and path not in turn["files"]:
                    turn["files"].append(path)
    lines = []
    for turn in turns.values():
        lines += [f"User: {_shorten(text)}" for text in turn["user"]]
        if turn["files"]:
            lines.append(f"(Assistant changed: {', '.join(turn['files'])})")
        if turn["reply"]:
            lines.append(f"Assistant: {_shorten(turn['reply'])}")
    text = "\n\n".join(lines)
    if len(text) > TRANSCRIPT_LIMIT:
        text = "…(earlier messages omitted)\n\n" + text[-TRANSCRIPT_LIMIT:]
    if not text:
        return ""
    return (
        "<forked_conversation>\nThis chat was forked from an earlier conversation "
        "about the same project. The files in the project folder are the current "
        "state. Earlier messages:\n\n" + text + "\n</forked_conversation>"
    )


def _thread_summary(thread: dict) -> dict:
    turns = thread.get("turns", [])
    last = turns[-1] if turns else None
    return {
        "id": thread["id"],
        "project": thread["project"],
        "workspace": thread.get("workspace"),
        "title": thread["title"],
        "provider": thread["provider"],
        "model": thread["model"],
        "effort": thread.get("effort", ""),
        "created": thread["created"],
        "updated": thread["updated"],
        "status": last["status"] if last else "idle",
        "locked": bool(thread.get("session")),
        "forked_from": thread.get("forked_from"),
    }


class AgentManager:
    def __init__(
        self, data_root: Path, *, python: str, app_root: Path, latex=lambda: True
    ):
        self.data_root = data_root
        self.python = python
        self.app_root = app_root
        self.latex = latex
        self.lock = threading.RLock()
        self.threads: dict[str, dict] = {}
        self.running: dict[str, Turn] = {}
        self.statuses: dict[str, dict] = {}
        self.probing = False
        self.probe_thread: threading.Thread | None = None
        self.settings = clean_settings(self._read(self.settings_path))
        self.on_turn_finished = None
        # Providers whose CLI is being updated; guarded by ``lock``.
        self.updating: set[str] = set()

    # ---- storage
    @property
    def settings_path(self) -> Path:
        return self.data_root / ".studio" / "agents.json"

    def thread_dir(self, project_id: str) -> Path:
        if not ID_PATTERN.fullmatch(project_id or ""):
            raise KeyError(project_id)
        return self.data_root / ".studio" / "threads" / project_id

    @staticmethod
    def _read(path: Path):
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError, RecursionError):
            return None

    def save(self, thread: dict):
        with self.lock:
            snapshot = json.loads(json.dumps(thread))
        _write_json(
            self.thread_dir(thread["project"]) / f"{thread['id']}.json", snapshot
        )

    def update_settings(self, value: dict) -> dict:
        with self.lock:
            merged = json.loads(json.dumps(self.settings))
            for key, item in value.items():
                if key == "providers" and isinstance(item, dict):
                    for provider, config in item.items():
                        if provider in merged["providers"] and isinstance(config, dict):
                            merged["providers"][provider].update(config)
                else:
                    merged[key] = item
            self.settings = clean_settings(merged)
            _write_json(self.settings_path, self.settings)
            settings = json.loads(json.dumps(self.settings))
        self.refresh()
        return settings

    # ---- providers
    def snapshot(self) -> dict:
        with self.lock:
            return {
                "providers": [
                    self.statuses.get(provider) or self._pending(provider)
                    for provider in PROVIDERS
                ],
                "settings": json.loads(json.dumps(self.settings)),
                "probing": self.probing,
            }

    def _pending(self, provider: str) -> dict:
        return {
            "id": provider,
            "name": PROVIDERS[provider]["name"],
            "enabled": self.settings["providers"][provider]["enabled"],
            "installed": False,
            "status": "checking",
            "message": "Checking…",
            "auth": {"status": "unknown"},
            "models": [],
            "install": PROVIDERS[provider]["install"],
            "docs": PROVIDERS[provider]["docs"],
        }

    def refresh(self, wait=False):
        with self.lock:
            if self.probe_thread is not None and self.probe_thread.is_alive():
                thread = self.probe_thread
            else:
                self.probing = True
                thread = threading.Thread(target=self._probe_all, daemon=True)
                self.probe_thread = thread
                thread.start()
        publish("providers", self.snapshot())
        if wait:
            thread.join(timeout=90)
        return self.snapshot()

    def _probe_all(self):
        try:
            with self.lock:
                settings = json.loads(json.dumps(self.settings))
            results = {}
            workers = [
                threading.Thread(
                    target=lambda p=p: results.__setitem__(
                        p, self._safe_probe(p, settings)
                    )
                )
                for p in PROVIDERS
            ]
            for worker in workers:
                worker.start()
            for worker in workers:
                worker.join(timeout=80)
            with self.lock:
                self.statuses.update(results)
        finally:
            with self.lock:
                self.probing = False
            publish("providers", self.snapshot())

    @staticmethod
    def _safe_probe(provider, settings):
        try:
            return probe(provider, settings)
        except Exception as error:  # A broken CLI must not take Studio down.
            logging.exception("Unable to check %s", provider)
            status = {
                "id": provider,
                "name": PROVIDERS[provider]["name"],
                "enabled": settings["providers"][provider]["enabled"],
                "installed": False,
                "status": "error",
                "message": f"Couldn't check {PROVIDERS[provider]['name']}: {error}",
                "auth": {"status": "unknown"},
                "models": [],
                "install": PROVIDERS[provider]["install"],
                "docs": PROVIDERS[provider]["docs"],
            }
            return status

    def open_login(self, provider: str):
        if provider not in PROVIDERS:
            raise AgentError("Choose Codex or Claude Code.")
        binary = resolve_binary(
            provider, self.settings["providers"][provider]["binary"]
        )
        if not binary:
            raise AgentError(f"Install {PROVIDERS[provider]['name']} first.")
        command = [*launch_command(binary), *PROVIDERS[provider]["login"]]
        if os.name == "nt":
            subprocess.Popen(command, creationflags=subprocess.CREATE_NEW_CONSOLE)
        else:
            subprocess.Popen(command, start_new_session=True)
        return {"ok": True}

    def open_install(self, provider: str):
        if provider not in PROVIDERS:
            raise AgentError("Choose Codex or Claude Code.")
        if os.name != "nt":
            raise AgentError(
                "Use the provider's setup documentation to install on this platform."
            )
        shell = shutil.which("powershell.exe")
        if not shell:
            raise AgentError(
                "PowerShell was not found. Use the setup documentation to install manually."
            )
        # Only these fixed provider commands are executable; never accept shell text from HTTP.
        script = "$ErrorActionPreference = 'Stop'; "
        if provider == "codex":
            script += "if (-not (Get-Command npm.cmd -ErrorAction SilentlyContinue)) { throw 'Install Node.js LTS from https://nodejs.org/en/download, reopen Studio, then install Codex.' }; "
        script += (
            "npm.cmd install -g @openai/codex; if ($LASTEXITCODE -ne 0) { throw 'Codex installation failed. Check the output above, then retry.' }"
            if provider == "codex"
            else PROVIDERS[provider]["install"]
        )
        script += "; Write-Host 'When installation finishes, return to Manim Studio and click Check again.'"
        try:
            subprocess.Popen(
                [shell, "-NoProfile", "-NoExit", "-Command", script],
                cwd=Path.home(),
                creationflags=subprocess.CREATE_NEW_CONSOLE,
            )
        except OSError as error:
            raise AgentError(f"Couldn't open the installer: {error}") from error
        return {"ok": True}

    # ---- threads
    def _load_project_threads(self, project_id: str):
        directory = self.thread_dir(project_id)
        if not directory.is_dir():
            return
        for path in directory.glob("*.json"):
            if not ID_PATTERN.fullmatch(path.stem) or path.stem in self.threads:
                continue
            thread = self._read(path)
            if (
                isinstance(thread, dict)
                and thread.get("id") == path.stem
                and thread.get("project") == project_id
                and isinstance(thread.get("items"), list)
                and thread.get("provider") in PROVIDERS
            ):
                thread.setdefault("turns", [])
                for turn in thread["turns"]:
                    if turn.get("status") == "running":
                        # Studio closed mid-turn; the CLI process ended with it.
                        turn.update(
                            status="interrupted",
                            finished=turn.get("finished") or _now(),
                        )
                self.threads[thread["id"]] = thread

    def list_threads(self, project_id: str) -> list[dict]:
        with self.lock:
            self._load_project_threads(project_id)
            threads = [
                _thread_summary(thread)
                for thread in self.threads.values()
                if thread["project"] == project_id
            ]
        return sorted(threads, key=lambda thread: thread["updated"], reverse=True)

    def get(self, project_id: str, thread_id: str) -> dict:
        with self.lock:
            self._load_project_threads(project_id)
            thread = self.threads.get(thread_id)
            if thread is None or thread["project"] != project_id:
                raise KeyError(thread_id)
            return thread

    def view(self, project_id: str, thread_id: str) -> dict:
        with self.lock:
            thread = self.get(project_id, thread_id)
            return {
                **_thread_summary(thread),
                "items": list(thread["items"]),
                "turns": list(thread["turns"]),
            }

    def _choose(self, provider, model, effort):
        provider = (
            provider
            or self.settings["default_provider"]
            or next(
                (
                    status["id"]
                    for status in (self.statuses.get(p) for p in PROVIDERS)
                    if status and status.get("status") == "ready"
                ),
                "codex",
            )
        )
        if provider not in PROVIDERS:
            raise AgentError("Choose Codex or Claude Code.")
        config = self.settings["providers"][provider]
        model = config["model"] if model is None else model
        effort = config["effort"] if effort is None else effort
        if model and not MODEL_PATTERN.fullmatch(model):
            raise AgentError("Choose a model from the list.")
        if effort and effort not in EFFORTS:
            raise AgentError("Choose a listed reasoning level.")
        return provider, model, effort

    def create(
        self,
        project_id: str,
        *,
        provider=None,
        model=None,
        effort=None,
        title="",
        workspace=None,
    ) -> dict:
        if workspace is not None:
            projects.workspace_folder(
                self.data_root / "projects", project_id, workspace
            )
        provider, model, effort = self._choose(provider, model, effort)
        self.thread_dir(project_id)
        thread = {
            "id": uuid.uuid4().hex,
            "project": project_id,
            "workspace": workspace,
            "title": (title or "").strip()[:80] or "New chat",
            "provider": provider,
            "model": model,
            "effort": effort,
            "session": None,
            "created": _now(),
            "updated": _now(),
            "items": [],
            "turns": [],
        }
        with self.lock:
            self.threads[thread["id"]] = thread
        self.save(thread)
        self.emit_meta(thread)
        return self.view(project_id, thread["id"])

    def update(
        self,
        project_id: str,
        thread_id: str,
        *,
        title=None,
        provider=None,
        model=None,
        effort=None,
    ):
        with self.lock:
            thread = self.get(project_id, thread_id)
            if title is not None:
                thread["title"] = title.strip()[:80] or thread["title"]
            if provider is not None and provider != thread["provider"]:
                if thread.get("session"):
                    raise AgentError("Start a new chat to switch to another agent.")
                if thread_id in self.running:
                    raise AgentError("Wait for the agent to finish.")
                thread["provider"], thread["model"], thread["effort"] = self._choose(
                    provider, None, None
                )
            if model is not None:
                thread["model"] = self._choose(
                    thread["provider"], model, thread["effort"]
                )[1]
            if effort is not None:
                thread["effort"] = self._choose(
                    thread["provider"], thread["model"], effort
                )[2]
            thread["updated"] = _now()
        self.save(thread)
        self.emit_meta(thread)
        return _thread_summary(thread)

    def fork(
        self,
        project_id: str,
        thread_id: str,
        *,
        turn_id=None,
        title="",
        provider=None,
        model=None,
        effort=None,
    ) -> dict:
        """Copy a chat (up to and including `turn_id`) into a new, independent one.

        Provider sessions cannot be split, so the new chat starts a fresh session
        and replays the copied conversation as context on its first message.
        """
        with self.lock:
            source = self.get(project_id, thread_id)
            turns = [dict(turn) for turn in source["turns"]]
            if turn_id is not None:
                position = next(
                    (i for i, turn in enumerate(turns) if turn["id"] == turn_id), None
                )
                if position is None:
                    raise AgentError("That message is no longer in this chat.")
                turns = turns[: position + 1]
            if not turns:
                raise AgentError("Send a message before forking this chat.")
            if any(turn.get("status") == "running" for turn in turns):
                raise AgentError("Wait for the agent to finish before forking.")
            included = {turn["id"] for turn in turns}
            items = [dict(item) for item in source["items"] if item["turn"] in included]
            provider_id, chosen_model, chosen_effort = self._choose(
                provider or source["provider"],
                model
                if model is not None
                else (
                    source["model"]
                    if not provider or provider == source["provider"]
                    else None
                ),
                effort
                if effort is not None
                else (
                    source["effort"]
                    if not provider or provider == source["provider"]
                    else None
                ),
            )
            for turn in turns:
                turn.pop("usage", None)
                turn.pop("tokens", None)
            now = _now()
            thread = {
                "id": uuid.uuid4().hex,
                "project": project_id,
                "title": (title or "").strip()[:80]
                or f"Fork of {source['title']}"[:80],
                "provider": provider_id,
                "model": chosen_model,
                "effort": chosen_effort,
                "session": None,
                "created": now,
                "updated": now,
                "items": items,
                "turns": turns,
                "forked_from": {
                    "thread": source["id"],
                    "title": source["title"],
                    "turn": turns[-1]["id"],
                },
                "fork_context": transcript(items),
                "workspace": source.get("workspace"),
            }
            self.threads[thread["id"]] = thread
        self.save(thread)
        self.emit_meta(thread)
        return self.view(project_id, thread["id"])

    def delete(self, project_id: str, thread_id: str):
        self.interrupt(project_id, thread_id, wait=True)
        with self.lock:
            self.get(project_id, thread_id)
            self.threads.pop(thread_id, None)
        (self.thread_dir(project_id) / f"{thread_id}.json").unlink(missing_ok=True)
        publish("thread.deleted", {"project": project_id, "thread": thread_id})

    def delete_project(self, project_id: str):
        with self.lock:
            ids = [
                tid
                for tid, thread in self.threads.items()
                if thread["project"] == project_id
            ]
        for thread_id in ids:
            self.interrupt(project_id, thread_id, wait=True)
            with self.lock:
                self.threads.pop(thread_id, None)
        try:
            shutil.rmtree(self.thread_dir(project_id), ignore_errors=True)
        except KeyError:
            pass

    def busy_project(self, project_id: str) -> bool:
        with self.lock:
            return any(
                turn.thread["project"] == project_id for turn in self.running.values()
            )

    def busy_workspace(self, project_id: str, workspace: str | None = None) -> bool:
        with self.lock:
            return any(
                turn.thread["project"] == project_id
                and (
                    workspace is None
                    or turn.thread.get("workspace") in {None, workspace}
                )
                for turn in self.running.values()
            )

    # ---- events
    def emit_item(self, thread: dict, item: dict, save=True):
        publish(
            "thread.item",
            {"project": thread["project"], "thread": thread["id"], "item": item},
        )
        if save:
            self.save(thread)

    def emit_meta(self, thread: dict):
        with self.lock:
            summary = _thread_summary(thread)
        publish("thread.meta", {"thread": summary})

    def emit_turn(self, thread: dict, turn: dict):
        publish(
            "thread.turn",
            {"project": thread["project"], "thread": thread["id"], "turn": dict(turn)},
        )

    # ---- turns
    def send(
        self,
        project_id: str,
        thread_id: str,
        text: str,
        *,
        model=None,
        effort=None,
        context="",
    ) -> dict:
        text = (text or "").strip()
        if not text:
            raise AgentError("Write a message for the agent.")
        if len(text) + len(context) > MAX_PROMPT:
            raise AgentError("Shorten the message and try again.")
        self._wait_for_update(project_id, thread_id)
        with self.lock:
            thread = self.get(project_id, thread_id)
            project_dir = projects.workspace_folder(
                self.data_root / "projects", project_id, thread.get("workspace")
            )
            if thread_id in self.running:
                raise AgentError("The agent is still working on this chat.")
            if self.busy_workspace(project_id, thread.get("workspace")):
                raise AgentError(
                    "Another chat is editing this project. Wait for it or stop it first."
                )
            if model is not None:
                thread["model"] = self._choose(
                    thread["provider"], model, thread["effort"]
                )[1]
            if effort is not None:
                thread["effort"] = self._choose(
                    thread["provider"], thread["model"], effort
                )[2]
            if thread["provider"] in self.updating:
                raise AgentError(
                    f"{PROVIDERS[thread['provider']]['name']} is updating. Try again in a minute."
                )
            status = self.statuses.get(thread["provider"])
            config = self.settings["providers"][thread["provider"]]
            if not config["enabled"]:
                raise AgentError(
                    f"{PROVIDERS[thread['provider']]['name']} is turned off in Settings."
                )
            binary = resolve_binary(thread["provider"], config["binary"])
            if not binary:
                raise AgentError(
                    f"{PROVIDERS[thread['provider']]['name']} isn't installed. Open Settings → Providers."
                )
            if status and status.get("auth", {}).get("status") == "unauthenticated":
                raise AgentError(
                    f"Sign in to {PROVIDERS[thread['provider']]['name']} in Settings → Providers."
                )
            fork_context = thread.get("fork_context") or ""
            if fork_context:
                context = f"{fork_context}\n\n{context}" if context else fork_context
                thread["fork_context"] = ""
            turn_record = {
                "id": uuid.uuid4().hex,
                "status": "running",
                "provider": thread["provider"],
                "model": thread["model"],
                "effort": thread["effort"],
                "started": _now(),
                "finished": None,
                "changed": [],
                "error": None,
            }
            thread["turns"].append(turn_record)
            if thread["title"] == "New chat":
                thread["title"] = re.sub(r"\s+", " ", text)[:60]
            thread["updated"] = _now()
            turn = Turn(self, thread, project_dir, turn_record)
            self.running[thread_id] = turn
        turn.upsert(f"user:{turn_record['id']}", kind="user", text=text)
        self.emit_turn(thread, turn_record)
        self.emit_meta(thread)
        worker = threading.Thread(
            target=self._run_turn, args=(turn, binary, text, context), daemon=True
        )
        try:
            worker.start()
        except RuntimeError:
            with self.lock:
                self.running.pop(thread_id, None)
                turn_record.update(
                    status="failed", finished=_now(), error="The agent could not start."
                )
            self.emit_turn(thread, turn_record)
            raise AgentError("The agent could not start. Try again.")
        return dict(turn_record)

    def _wait_for_update(self, project_id: str, thread_id: str):
        """Hold a new turn briefly while its provider's CLI is being updated."""
        deadline = time.monotonic() + UPDATE_WAIT
        while True:
            with self.lock:
                provider = self.get(project_id, thread_id)["provider"]
                if provider not in self.updating or time.monotonic() >= deadline:
                    return
            time.sleep(0.2)

    def interrupt(self, project_id: str, thread_id: str, wait=False):
        with self.lock:
            turn = self.running.get(thread_id)
            if turn is None or turn.thread["project"] != project_id:
                return {"ok": True}
            turn.cancelled = True
            process = turn.process
        try:
            stop_process(process)
        except (OSError, subprocess.SubprocessError):
            logging.exception("Unable to stop the agent")
            if process is not None and process.poll() is None:
                process.kill()
        if wait:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                with self.lock:
                    if thread_id not in self.running:
                        break
                time.sleep(0.05)
        return {"ok": True}

    def shutdown(self):
        with self.lock:
            running = [
                (turn.thread["project"], tid) for tid, turn in self.running.items()
            ]
        for project_id, thread_id in running:
            self.interrupt(project_id, thread_id, wait=True)

    def check_command(self) -> str:
        return f'"{self.python}" "{self.app_root / "agent_check.py"}"'

    def build_command(
        self, turn: Turn, binary: str, prompt_dir: Path
    ) -> tuple[list[str], str]:
        thread = turn.thread
        session = thread.get("session")
        resume = (
            session["id"]
            if session and session.get("provider") == thread["provider"]
            else None
        )
        full_access = self.settings["access"] == "full"
        system = instructions(
            turn.project_dir,
            python=self.python,
            check=self.check_command(),
            latex=bool(self.latex()),
        )
        if thread["provider"] == "codex":
            # `exec resume` accepts fewer flags, so policy goes through -c overrides.
            options = ["--json", "--skip-git-repo-check"]
            options += (
                ["--dangerously-bypass-approvals-and-sandbox"]
                if full_access
                else [
                    "-c",
                    'sandbox_mode="workspace-write"',
                    "-c",
                    'approval_policy="never"',
                ]
            )
            for value in codex_overrides(binary):
                options += ["-c", value]
            if thread["model"]:
                options += ["-m", thread["model"]]
            if thread["effort"]:
                options += ["-c", f'model_reasoning_effort="{thread["effort"]}"']
            command = (
                [*launch_command(binary), "exec", "resume", *options, resume, "-"]
                if resume
                else [
                    *launch_command(binary),
                    "exec",
                    *options,
                    "-C",
                    str(turn.project_dir),
                    "-",
                ]
            )
            # Codex has no system-prompt file flag; brief it once per session.
            preamble = (
                ""
                if resume
                else f"<studio_instructions>\n{system}\n</studio_instructions>\n\n"
            )
            return command, preamble
        system_file = prompt_dir / "system.md"
        system_file.write_text(system, encoding="utf-8")
        command = [
            *launch_command(binary),
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--permission-mode",
            "bypassPermissions" if full_access else "acceptEdits",
            "--append-system-prompt-file",
            str(system_file),
            *claude_options(binary, prompt_dir),
        ]
        if not full_access:
            command += [
                "--allowedTools",
                "Read",
                "Write",
                "Edit",
                "MultiEdit",
                "Glob",
                "Grep",
                "Bash",
                "PowerShell",
            ]
        if thread["model"]:
            command += ["--model", thread["model"]]
        if thread["effort"] in ("low", "medium", "high", "xhigh", "max"):
            command += ["--effort", thread["effort"]]
        if resume:
            command += ["--resume", resume]
        return command, ""

    def _snapshot_files(self, folder: Path) -> dict[str, tuple[int, int]]:
        files = {}
        for name in (projects.SCRIPT_FILE, projects.NARRATION_FILE):
            try:
                stat = (folder / name).stat()
                files[name] = (stat.st_mtime_ns, stat.st_size)
            except OSError:
                pass
        for asset in projects.list_assets(folder / projects.ASSETS):
            try:
                stat = (folder / projects.ASSETS / asset["path"]).stat()
                files[f"assets/{asset['path']}"] = (stat.st_mtime_ns, stat.st_size)
            except OSError:
                pass
        return files

    def _run_turn(self, turn: Turn, binary: str, text: str, context: str):
        thread, record = turn.thread, turn.record
        before = self._snapshot_files(turn.project_dir)
        stderr_tail: list[str] = []
        prompt_dir = Path(tempfile.mkdtemp(prefix="turn-", dir=self._scratch()))
        process = None
        try:
            command, preamble = self.build_command(turn, binary, prompt_dir)
            prompt = (
                preamble + (f"{context.strip()}\n\n" if context.strip() else "") + text
            )
            env = dict(os.environ, PYTHONIOENCODING="utf-8", NO_COLOR="1")
            if thread["provider"] == "claude":
                env.update(CLAUDE_ENV)
            with self.lock:
                if turn.cancelled:
                    raise InterruptedError
                process = subprocess.Popen(
                    command,
                    cwd=turn.project_dir,
                    env=env,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    encoding="utf-8",
                    errors="replace",
                    **popen_options(),
                )
                turn.process = process
                try:
                    attach_process_tree(process)
                except OSError:
                    logging.exception("Unable to attach the agent process tree")

            def drain():
                for line in process.stderr:
                    stderr_tail.append(line)
                    while sum(map(len, stderr_tail)) > MAX_STDERR:
                        stderr_tail.pop(0)

            reader = threading.Thread(target=drain, daemon=True)
            reader.start()
            try:
                process.stdin.write(prompt)
                process.stdin.close()
            except OSError:
                pass  # The CLI exited early; its exit code explains why.
            stream = (
                CodexStream(turn)
                if thread["provider"] == "codex"
                else ClaudeStream(turn)
            )
            for line in process.stdout:
                line = line.strip()
                if not line.startswith("{"):
                    continue
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if isinstance(event, dict):
                    try:
                        stream.handle(event)
                    except Exception:  # One odd event must not end the turn.
                        logging.exception("Unable to read an agent event")
            code = process.wait()
            reader.join(timeout=5)
            if turn.cancelled:
                raise InterruptedError
            if code and not turn.error:
                detail = "".join(stderr_tail).strip().splitlines()
                useful = [
                    line for line in detail if line.strip() and "WARN" not in line[:40]
                ]
                turn.fail(
                    (useful or detail or [f"The agent exited with code {code}."])[-1][
                        :1000
                    ]
                )
            status = "failed" if turn.error else "completed"
        except InterruptedError:
            status = "interrupted"
        except (OSError, subprocess.SubprocessError) as error:
            turn.fail(
                f"Couldn't start {PROVIDERS[thread['provider']]['name']}: {error}"
            )
            status = "failed"
        finally:
            if process is not None:
                try:
                    stop_process(process)
                except (OSError, subprocess.SubprocessError):
                    logging.exception("Unable to stop the agent process tree")
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                try:
                    if not tree_active(process):
                        release_process_tree(process)
                except OSError:
                    logging.exception("Unable to release the agent process tree")
                for stream_handle in (process.stdout, process.stderr):
                    try:
                        if stream_handle is not None:
                            stream_handle.close()
                    except OSError:
                        pass
            shutil.rmtree(prompt_dir, ignore_errors=True)
        if status == "interrupted":
            turn.upsert(f"notice:{record['id']}", kind="notice", text="Stopped.")
        elif turn.error:
            turn.upsert(
                f"error:{record['id']}",
                kind="error",
                text=turn.error,
                **(turn.error_info or {}),
            )
        # Close any rows that were still open when the process ended.
        with self.lock:
            unfinished = {
                key: "status" in item
                for key, item in turn.items_by_key.items()
                if item.get("status") == "in_progress" or item.get("streaming")
            }
        for key, has_status in unfinished.items():
            fields = {"streaming": False}
            if has_status:
                fields["status"] = "completed" if status == "completed" else "failed"
            turn.upsert(key, **fields)
        after = self._snapshot_files(turn.project_dir)
        changed = sorted(
            name
            for name in set(before) | set(after)
            if before.get(name) != after.get(name)
        )
        if projects.NARRATION_FILE in changed:
            self._enable_narration(thread["project"], thread.get("workspace"))
        with self.lock:
            record.update(
                status=status,
                finished=_now(),
                changed=changed,
                error=turn.error,
                error_code=(turn.error_info or {}).get("code"),
                usage=turn.usage,
                tokens=token_usage(turn.usage),
            )
            thread["updated"] = _now()
            self.running.pop(thread["id"], None)
        self.save(thread)
        self.emit_turn(thread, record)
        self.emit_meta(thread)
        if changed:
            publish(
                "project.changed",
                {
                    "project": thread["project"],
                    "files": changed,
                    "thread": thread["id"],
                },
            )
        if self.on_turn_finished:
            try:
                self.on_turn_finished(thread, dict(record))
            except Exception:
                logging.exception("Unable to continue the video workflow")

    def _enable_narration(self, project_id: str, workspace: str | None = None):
        try:
            project = projects.read(self.data_root / "projects", project_id, workspace)
        except (KeyError, OSError):
            return
        narration = dict(project["narration"])
        if narration["text"].strip() and not narration["enabled"]:
            narration["enabled"] = True
            try:
                projects.update(
                    self.data_root / "projects",
                    project_id,
                    narration=narration,
                    workspace=workspace,
                )
            except (KeyError, OSError, projects.ProjectError):
                logging.exception("Unable to turn on generated narration")

    def _scratch(self) -> Path:
        path = self.data_root / ".studio" / "agent-runs"
        path.mkdir(parents=True, exist_ok=True)
        return path
