"""Keep the installed agent CLIs (Codex, Claude Code) up to date.

Studio never installs a provider itself; it only updates a CLI the user already
installed, through the same channel that installed it:

* npm installs run ``npm install -g <package>@latest`` with the CLI's own
  global prefix, so the copy Studio launches is the one that changes;
* the native Claude Code installer runs ``claude update``;
* the native Codex installer runs ``codex update``.

Anything else (a custom binary, Homebrew, WinGet, a wrapper script) is reported
as managed externally. The latest version comes from the public npm registry.
Updates wait until no turn for that provider is running, and new turns for the
provider wait for (or are refused during) the short update window. Every
failure is recorded as a status; nothing here may stop Studio.
"""

from __future__ import annotations

from datetime import datetime, timedelta, timezone
import json
import logging
import os
from pathlib import Path
import re
import shutil
import subprocess
import threading
import time
import urllib.error
import urllib.request

import agents
from events import publish as publish_event
from processes import (
    attach_process_tree,
    popen_options,
    release_process_tree,
    stop_process,
)

PACKAGES = {"codex": "@openai/codex", "claude": "@anthropic-ai/claude-code"}
REGISTRY = "https://registry.npmjs.org/{package}/latest"
STATE_FILE = Path(".studio") / "agent-updates.json"
CHECK_INTERVAL = timedelta(hours=24)
STARTUP_DELAY = 10
DEFERRED_POLL = 15
NETWORK_TIMEOUT = 10
MAX_REGISTRY_BYTES = 2 * 1024 * 1024
UPDATE_TIMEOUT = 10 * 60
OUTPUT_TAIL = 4000
# Set to 0 to turn off the automatic background check (manual checks still work).
ENVIRONMENT_SWITCH = "MANIM_STUDIO_AGENT_UPDATES"
_VERSION = re.compile(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?")


class AgentUpdateError(Exception):
    """A user-facing reason an update cannot run."""


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _parse_time(value) -> datetime | None:
    try:
        moment = datetime.fromisoformat(value)
    except (TypeError, ValueError):
        return None
    return moment if moment.tzinfo else moment.replace(tzinfo=timezone.utc)


# ---------------------------------------------------------------- versions


def parse_version(text) -> tuple | None:
    """A sortable key for a semantic version found in ``text``."""
    match = _VERSION.search(str(text or ""))
    if not match:
        return None
    release = tuple(int(part) for part in match.groups()[:3])
    pre = match[4]
    if pre is None:
        # A release sorts after every prerelease of the same version.
        return (*release, 1, ())
    parts = tuple(
        (0, int(part), "") if part.isdigit() else (1, 0, part)
        for part in pre.split(".")
    )
    return (*release, 0, parts)


def compare_versions(left, right) -> int | None:
    """-1, 0 or 1 like ``cmp``; None when either side is not a version."""
    a, b = parse_version(left), parse_version(right)
    if a is None or b is None:
        return None
    return (a > b) - (a < b)


def latest_version(provider: str) -> str:
    """The newest published version of the provider's npm package."""
    url = REGISTRY.format(package=PACKAGES[provider].replace("/", "%2F", 1))
    if not url.startswith("https://registry.npmjs.org/"):
        raise AgentUpdateError("Unexpected registry address.")
    request = urllib.request.Request(
        url,
        headers={
            "User-Agent": "Manim-Studio-Agent-Updates",
            "Accept": "application/json",
            "Accept-Encoding": "identity",
        },
    )
    with urllib.request.urlopen(request, timeout=NETWORK_TIMEOUT) as response:
        if not response.geturl().startswith("https://registry.npmjs.org/"):
            raise AgentUpdateError("The npm registry redirected unexpectedly.")
        payload = response.read(MAX_REGISTRY_BYTES + 1)
    if len(payload) > MAX_REGISTRY_BYTES:
        raise AgentUpdateError("The npm registry returned an unexpected response.")
    data = json.loads(payload.decode("utf-8"))
    version = data.get("version") if isinstance(data, dict) else None
    if not isinstance(version, str) or parse_version(version) is None:
        raise AgentUpdateError("The npm registry did not report a version.")
    return version[:60]


# ---------------------------------------------------------------- install kind


def _inside(path: Path, folder: Path) -> bool:
    """Whether ``path`` (as launched, or after following links) is in ``folder``."""
    try:
        root = folder.resolve()
        return path.absolute().is_relative_to(root) or path.resolve().is_relative_to(
            root
        )
    except (OSError, ValueError):
        return False


def _npm_prefix(provider: str, binary: Path) -> Path | None:
    """The global npm prefix that owns ``binary``, if npm installed it."""
    package = PACKAGES[provider].split("/")
    candidates = [
        # Windows: <prefix>\codex.cmd beside <prefix>\node_modules\@openai\codex
        (binary.parent, binary.parent / "node_modules"),
        # POSIX: <prefix>/bin/codex beside <prefix>/lib/node_modules/@openai/codex
        (binary.parent.parent, binary.parent.parent / "lib" / "node_modules"),
    ]
    for prefix, modules in candidates:
        if (modules.joinpath(*package) / "package.json").is_file():
            if prefix == binary.parent or binary.parent.name == "bin":
                return prefix
    try:
        real = binary.resolve()
    except OSError:
        return None
    parts = real.parts
    for index in range(len(parts) - len(package)):
        if parts[index] == "node_modules" and list(
            parts[index + 1 : index + 1 + len(package)]
        ) == list(package):
            modules = Path(*parts[: index + 1])
            if modules.parent.name == "lib":
                return modules.parent.parent
            return modules.parent
    return None


def install_method(provider: str, binary: str | None, npm: str | None = None) -> dict:
    """How the CLI at ``binary`` can be updated without guessing.

    Returns ``{"method": "npm"|"native"|"external"|"missing", "command": [...]}``.
    """
    if not binary:
        return {"method": "missing", "command": []}
    path = Path(binary)
    prefix = _npm_prefix(provider, path)
    if prefix is not None:
        npm = npm if npm is not None else shutil.which("npm")
        if not npm:
            return {
                "method": "npm",
                "command": [],
                "problem": "npm was not found, so this npm install can't be updated.",
            }
        return {
            "method": "npm",
            "command": [
                npm,
                "install",
                "--global",
                "--prefix",
                str(prefix),
                "--no-fund",
                "--no-audit",
                f"{PACKAGES[provider]}@latest",
            ],
        }
    home = Path.home()
    if provider == "claude":
        native = (
            path.parent == home / ".local" / "bin"
            and path.name.lower() in {"claude", "claude.exe"}
            and (home / ".local" / "share" / "claude").is_dir()
        ) or _inside(path, home / ".local" / "share" / "claude")
        legacy = _inside(path, home / ".claude" / "local")
        if native or legacy:
            return {"method": "native", "command": [binary, "update"]}
    if provider == "codex":
        # The standalone installer links its bin folder to ~/.codex/packages.
        local = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
        if (
            os.name == "nt" and _inside(path, local / "Programs" / "OpenAI" / "Codex")
        ) or _inside(path, home / ".codex" / "packages" / "standalone"):
            return {"method": "native", "command": [binary, "update"]}
    return {"method": "external", "command": []}


def _tail(text: str, limit=OUTPUT_TAIL) -> str:
    text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", text or "").strip()
    return text if len(text) <= limit else "…" + text[-limit:]


# ---------------------------------------------------------------- updater


class AgentUpdater:
    """Background checks and serialized, deferred CLI updates."""

    def __init__(
        self,
        manager,
        *,
        publish=publish_event,
        fetch_latest=latest_version,
        method=install_method,
        installed_version=None,
        automatic: bool | None = None,
    ):
        # ``manager`` is a callable so tests and Studio can swap the manager.
        self.manager = manager
        self.publish = publish
        self.fetch_latest = fetch_latest
        self.method = method
        self.installed_version = installed_version or self._installed_version
        self.automatic = (
            os.environ.get(ENVIRONMENT_SWITCH, "1") != "0"
            if automatic is None
            else automatic
        )
        self.lock = threading.Lock()
        self.wake_event = threading.Event()
        self.stop_event = threading.Event()
        self.thread: threading.Thread | None = None
        self.process: subprocess.Popen | None = None
        self.started_at = 0.0
        self.loaded = False
        self.last_check: str | None = None
        self.checking = False
        self.check_requested = False
        self.pending: set[str] = set()
        self.providers: dict[str, dict] = {
            provider: self._blank(provider) for provider in PACKAGES
        }

    @staticmethod
    def _blank(provider: str) -> dict:
        return {
            "provider": provider,
            "status": "unknown",
            "message": "",
            "installed": None,
            "latest": None,
            "available": False,
            "method": None,
            "checked": None,
            "updated": None,
            "output": "",
        }

    # ---- state
    def _state_path(self) -> Path:
        return Path(self.manager().data_root) / STATE_FILE

    def _settings(self) -> dict:
        manager = self.manager()
        with manager.lock:
            return json.loads(json.dumps(manager.settings))

    def auto_enabled(self) -> bool:
        return bool(self._settings().get("auto_update_agents", True))

    def load(self):
        try:
            saved = json.loads(self._state_path().read_text(encoding="utf-8"))
        except (OSError, ValueError, RecursionError):
            saved = None
        with self.lock:
            self.loaded = True
            if not isinstance(saved, dict):
                return
            if _parse_time(saved.get("last_check")):
                self.last_check = saved["last_check"]
            for provider, entry in (saved.get("providers") or {}).items():
                if provider not in self.providers or not isinstance(entry, dict):
                    continue
                current = self.providers[provider]
                for key in ("installed", "latest", "method", "checked", "updated"):
                    if isinstance(entry.get(key), str):
                        current[key] = entry[key][:200]
                if isinstance(entry.get("output"), str):
                    current["output"] = entry["output"][-OUTPUT_TAIL:]
                status = entry.get("status")
                # A run that was interrupted by shutdown is checked again.
                if status in {"current", "available", "external", "error", "missing"}:
                    current["status"] = status
                    current["message"] = str(entry.get("message") or "")[:500]
                current["available"] = bool(
                    compare_versions(current["latest"], current["installed"]) == 1
                )

    def _save(self):
        with self.lock:
            value = {
                "last_check": self.last_check,
                "providers": json.loads(json.dumps(self.providers)),
            }
        try:
            agents._write_json(self._state_path(), value)
        except OSError:
            logging.exception("Unable to save the agent update state")

    def snapshot(self) -> dict:
        try:
            auto = self.auto_enabled()
        except Exception:  # The manager may be mid-replacement in tests.
            auto = True
        with self.lock:
            return {
                "auto": auto,
                "checking": self.checking,
                "last_check": self.last_check,
                "providers": json.loads(json.dumps(self.providers)),
            }

    def emit(self):
        try:
            self.publish("agent.updates", {"updates": self.snapshot()})
        except Exception:
            logging.exception("Unable to publish agent update status")

    def _set(self, provider: str, **fields):
        with self.lock:
            entry = self.providers[provider]
            entry.update(fields)
            entry["available"] = bool(
                compare_versions(entry["latest"], entry["installed"]) == 1
            )
        self.emit()

    # ---- lifecycle
    def start(self):
        with self.lock:
            if self.thread is not None and self.thread.is_alive():
                return
            self.stop_event.clear()
            self.started_at = time.monotonic()
            self.thread = threading.Thread(
                target=self._loop, name="agent-updates", daemon=True
            )
            thread = self.thread
        if not self.loaded:
            self.load()
        thread.start()

    def wake(self):
        self.wake_event.set()

    def shutdown(self, timeout=10):
        self.stop_event.set()
        self.wake_event.set()
        with self.lock:
            process = self.process
            thread = self.thread
        if process is not None:
            try:
                stop_process(process)
            except (OSError, subprocess.SubprocessError):
                logging.exception("Unable to stop the agent CLI update")
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout)

    # ---- requests
    def check_now(self) -> dict:
        with self.lock:
            self.check_requested = True
            self.checking = True  # Shown at once; the worker clears it.
        self.start()
        self.wake()
        return self.snapshot()

    def update_now(self, provider: str) -> dict:
        if provider not in PACKAGES:
            raise AgentUpdateError("Choose Codex or Claude Code.")
        settings = self._settings()["providers"][provider]
        name = agents.PROVIDERS[provider]["name"]
        if not settings["enabled"]:
            raise AgentUpdateError(f"{name} is turned off in Settings.")
        binary = agents.resolve_binary(provider, settings["binary"])
        method = self.method(provider, binary)
        if method["method"] == "missing":
            raise AgentUpdateError(f"{name} isn't installed.")
        if method["method"] == "external":
            raise AgentUpdateError(
                f"This {name} install is managed externally. Update it with the tool that installed it."
            )
        if method.get("problem"):
            raise AgentUpdateError(method["problem"])
        with self.lock:
            self.pending.add(provider)
        self.start()
        self.wake()
        return self.snapshot()

    # ---- worker
    def _due(self) -> bool:
        if not self.automatic or time.monotonic() - self.started_at < STARTUP_DELAY:
            return False
        try:
            if not self.auto_enabled():
                return False
        except Exception:
            return False
        with self.lock:
            last = _parse_time(self.last_check)
        return last is None or datetime.now(timezone.utc) - last >= CHECK_INTERVAL

    def _loop(self):
        while not self.stop_event.is_set():
            try:
                with self.lock:
                    requested = self.check_requested
                    self.check_requested = False
                if requested or self._due():
                    self.check_all()
                self._run_pending()
            except Exception:  # Never let the updater take Studio down.
                logging.exception("Agent update loop failed")
            with self.lock:
                waiting = bool(self.pending)
            elapsed = time.monotonic() - self.started_at
            if waiting:
                timeout = DEFERRED_POLL
            elif self.automatic and elapsed < STARTUP_DELAY:
                timeout = max(0.5, STARTUP_DELAY - elapsed)
            else:
                timeout = 3600.0
            self.wake_event.wait(timeout)
            self.wake_event.clear()

    @staticmethod
    def _installed_version(binary: str) -> str | None:
        result = agents._run([binary, "--version"])
        return agents._version(result.stdout + "\n" + result.stderr)

    def check_all(self):
        """Compare each enabled CLI with the newest published version."""
        with self.lock:
            self.checking = True
        self.emit()
        try:
            settings = self._settings()
            for provider in PACKAGES:
                if self.stop_event.is_set():
                    return
                self._check(provider, settings)
            with self.lock:
                self.last_check = _now()
                if settings.get("auto_update_agents", True):
                    for provider, entry in self.providers.items():
                        if (
                            entry["available"]
                            and entry["method"] in {"npm", "native"}
                            and entry["status"] == "available"
                        ):
                            self.pending.add(provider)
        finally:
            with self.lock:
                self.checking = False
            self._save()
            self.emit()

    def _check(self, provider: str, settings: dict):
        config = settings["providers"][provider]
        name = agents.PROVIDERS[provider]["name"]
        if not config["enabled"]:
            self._set(
                provider,
                status="disabled",
                message=f"{name} is turned off.",
                checked=_now(),
            )
            return
        with self.lock:
            busy = self.providers[provider]["status"] in {"updating", "waiting"}
        if busy:
            return
        self._set(provider, status="checking", message="Checking for updates…")
        binary = agents.resolve_binary(provider, config["binary"])
        method = self.method(provider, binary)
        if method["method"] == "missing":
            self._set(
                provider,
                status="missing",
                message=f"{name} isn't installed.",
                method="missing",
                installed=None,
                checked=_now(),
            )
            return
        try:
            installed = self.installed_version(binary)
        except (OSError, subprocess.SubprocessError) as error:
            installed = None
            logging.info("Unable to read the %s version: %s", provider, error)
        try:
            latest = self.fetch_latest(provider)
        except (
            OSError,
            ValueError,
            AgentUpdateError,
            urllib.error.URLError,
            TimeoutError,
        ) as error:
            reason = getattr(error, "reason", None) or error
            self._set(
                provider,
                status="error",
                message=f"Couldn't check for updates: {reason}",
                installed=installed,
                method=method["method"],
                checked=_now(),
            )
            return
        newer = compare_versions(latest, installed) == 1
        fields = dict(installed=installed, latest=latest, method=method["method"])
        if method["method"] == "external":
            message = "Managed externally. Update it with the tool that installed it."
            if newer:
                message = f"Version {latest} is available. {message}"
            self._set(
                provider, status="external", message=message, checked=_now(), **fields
            )
        elif method.get("problem"):
            self._set(
                provider,
                status="error",
                message=method["problem"],
                checked=_now(),
                **fields,
            )
        elif newer:
            self._set(
                provider,
                status="available",
                message=f"Version {latest} is available.",
                checked=_now(),
                **fields,
            )
        elif installed is None:
            self._set(
                provider,
                status="error",
                message=f"Couldn't read the installed {name} version.",
                checked=_now(),
                **fields,
            )
        else:
            self._set(
                provider,
                status="current",
                message="Up to date.",
                checked=_now(),
                **fields,
            )

    def provider_busy(self, provider: str) -> bool:
        manager = self.manager()
        with manager.lock:
            return any(
                turn.record.get("provider") == provider
                for turn in manager.running.values()
            )

    def _run_pending(self):
        with self.lock:
            pending = sorted(self.pending)
        for provider in pending:
            if self.stop_event.is_set():
                return
            self._update(provider)

    def _update(self, provider: str):
        """Update one CLI now, or leave it pending while its turns run."""
        name = agents.PROVIDERS[provider]["name"]
        manager = self.manager()
        updating = getattr(manager, "updating", None)
        with manager.lock:
            busy = any(
                turn.record.get("provider") == provider
                for turn in manager.running.values()
            )
            if not busy and updating is not None:
                # From here new turns for this provider wait for the update.
                updating.add(provider)
        if busy:
            with self.lock:
                waiting = self.providers[provider]["status"] == "waiting"
            if not waiting:
                self._set(
                    provider,
                    status="waiting",
                    message=f"Will update when the running {name} chat finishes.",
                )
            return
        try:
            self._install(provider, name)
        finally:
            with self.lock:
                self.pending.discard(provider)
            if updating is not None:
                with manager.lock:
                    updating.discard(provider)
            self._save()
            self.emit()
        agents.forget_capabilities()  # The new version may accept other flags.
        try:
            manager.refresh()  # Re-probe so version, status and models refresh.
        except Exception:
            logging.exception("Unable to refresh providers after an update")

    def _install(self, provider: str, name: str):
        settings = self._settings()["providers"][provider]
        binary = agents.resolve_binary(provider, settings["binary"])
        method = self.method(provider, binary)
        if method["method"] not in {"npm", "native"} or not method["command"]:
            self._set(
                provider,
                status="external" if method["method"] == "external" else "error",
                message=method.get("problem")
                or "Managed externally. Update it with the tool that installed it.",
                method=method["method"],
            )
            return
        with self.lock:
            before = self.providers[provider].get("installed")
        self._set(provider, status="updating", message=f"Updating {name}…", output="")
        try:
            code, output = self._execute(method["command"])
        except (OSError, subprocess.SubprocessError) as error:
            self._set(
                provider,
                status="error",
                message=f"The update couldn't start: {error}",
                output="",
            )
            return
        if self.stop_event.is_set():
            self._set(
                provider, status="unknown", message="Update stopped.", output=output
            )
            return
        try:
            installed = self.installed_version(binary) if binary else None
        except (OSError, subprocess.SubprocessError):
            installed = None
        if code is None:
            self._set(
                provider,
                status="error",
                message="The update took too long and was stopped.",
                output=output,
                installed=installed or before,
            )
            return
        if code != 0:
            hint = (
                " Studio may need permission to change this install."
                if re.search(r"EACCES|EPERM|permission|access is denied", output, re.I)
                else ""
            )
            self._set(
                provider,
                status="error",
                message=f"The update failed (exit code {code}).{hint}",
                output=output,
                installed=installed or before,
            )
            return
        with self.lock:
            latest = self.providers[provider]["latest"]
        if latest and compare_versions(latest, installed) == 1:
            status, message = "available", f"Version {latest} is still available."
        else:
            status, message = "current", "Up to date."
        self._set(
            provider,
            status=status,
            message=message,
            output=output,
            installed=installed,
            updated=_now(),
        )

    def _execute(self, command: list[str]) -> tuple[int | None, str]:
        """Run ``command`` hidden and non-interactive; None means it timed out."""
        env = dict(
            os.environ,
            NO_COLOR="1",
            npm_config_yes="true",
            npm_config_update_notifier="false",
            npm_config_fund="false",
            npm_config_audit="false",
        )
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            env=env,
            **popen_options(),
        )
        try:
            attach_process_tree(process)
        except OSError:
            logging.exception("Unable to attach the update process tree")
        with self.lock:
            self.process = process
        lines: list[str] = []

        def drain():
            for line in process.stdout:
                lines.append(line)
                del lines[:-200]

        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        code: int | None
        try:
            code = process.wait(UPDATE_TIMEOUT)
        except subprocess.TimeoutExpired:
            code = None
        finally:
            if code is None or self.stop_event.is_set():
                try:
                    stop_process(process)
                except (OSError, subprocess.SubprocessError):
                    logging.exception("Unable to stop the agent CLI update")
                    process.kill()
                try:
                    process.wait(5)
                except subprocess.TimeoutExpired:
                    logging.error("The agent CLI update did not stop")
            reader.join(5)
            process.stdout.close()
            with self.lock:
                self.process = None
            try:
                release_process_tree(process)
            except OSError:
                logging.exception("Unable to close the update process job")
        return code, _tail("".join(lines))
