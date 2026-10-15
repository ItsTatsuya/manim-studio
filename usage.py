"""Plan usage limits for the signed-in Codex and Claude Code CLIs.

Each CLI reports its own limits, so Studio never reads a credential file or
calls a provider API itself:

* Codex: ``codex app-server`` answers ``account/rateLimits/read`` over stdio.
* Claude Code: ``claude -p /usage`` prints the plan's session and weekly use.

Results are cached for a minute because asking Claude starts a process.
"""

from __future__ import annotations

from datetime import datetime, timedelta
import json
import logging
import math
import os
import re
import subprocess
import threading
import time

import agents

CACHE_SECONDS = 60
CODEX_TIMEOUT = 20
CLAUDE_TIMEOUT = 40
PROVIDER_ORDER = ("codex", "claude")

_LOCK = threading.Lock()
_CACHE: dict | None = None
_CACHED_AT = 0.0


def _window(window_id, label, used, resets_at=None, minutes=None):
    used = max(0, min(100, int(round(used))))
    return {
        "id": window_id,
        "label": label,
        "used_percent": used,
        "left_percent": 100 - used,
        "resets_at": resets_at,
        "window_minutes": minutes,
    }


def _window_label(minutes):
    if minutes is None:
        return "Limit"
    if minutes >= 7 * 24 * 60:
        return "Weekly"
    if minutes >= 24 * 60:
        return f"{round(minutes / (24 * 60))}-day"
    if minutes <= 5 * 60:
        return "Session"
    return f"{round(minutes / 60)}-hour"


# ---------------------------------------------------------------- Codex


def parse_codex_limits(result) -> dict:
    """Read a ``GetAccountRateLimitsResponse`` into display windows."""
    limits = result.get("rateLimits") if isinstance(result, dict) else None
    if not isinstance(limits, dict):
        raise ValueError("Codex did not report usage limits.")
    windows = []
    for key in ("primary", "secondary"):
        item = limits.get(key)
        if not isinstance(item, dict) or "usedPercent" not in item:
            continue
        used = item["usedPercent"]
        if (
            isinstance(used, bool)
            or not isinstance(used, (int, float))
            or not math.isfinite(used)
        ):
            continue
        minutes = item.get("windowDurationMins")
        minutes = minutes if isinstance(minutes, int) else None
        resets = item.get("resetsAt")
        windows.append(
            _window(
                key if minutes is None else f"{minutes}m",
                _window_label(minutes),
                used,
                resets if isinstance(resets, int) else None,
                minutes,
            )
        )
    if not windows:
        raise ValueError("Codex did not report usage limits.")
    plan = limits.get("planType")
    return {
        "plan": plan if isinstance(plan, str) and plan != "unknown" else None,
        "windows": windows,
    }


def _codex(binary: str) -> dict:
    process = subprocess.Popen(
        [*agents.launch_command(binary), "app-server"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        errors="replace",
        **({"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}),
    )
    replies: dict[int, dict] = {}
    changed = threading.Condition()

    def read():
        for line in process.stdout:
            try:
                message = json.loads(line)
            except ValueError:
                continue
            if isinstance(message, dict) and isinstance(message.get("id"), int):
                with changed:
                    replies[message["id"]] = message
                    changed.notify_all()

    threading.Thread(target=read, daemon=True).start()

    def request(request_id, method, params):
        process.stdin.write(
            json.dumps(
                {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
            )
            + "\n"
        )
        process.stdin.flush()
        deadline = time.monotonic() + CODEX_TIMEOUT
        with changed:
            while request_id not in replies:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or process.poll() is not None:
                    raise TimeoutError("Codex did not answer in time.")
                changed.wait(min(remaining, 0.5))
            return replies[request_id]

    try:
        request(
            1,
            "initialize",
            {
                "clientInfo": {
                    "name": "manim-studio",
                    "title": "Manim Studio",
                    "version": "1",
                }
            },
        )
        process.stdin.write(
            json.dumps({"jsonrpc": "2.0", "method": "initialized"}) + "\n"
        )
        process.stdin.flush()
        reply = request(2, "account/rateLimits/read", None)
    finally:
        try:
            process.stdin.close()
        except OSError:
            pass
        try:
            process.terminate()
            process.wait(timeout=3)
        except (OSError, subprocess.SubprocessError):
            process.kill()
    if "error" in reply:
        raise ValueError("Codex could not read your usage limits.")
    return parse_codex_limits(reply.get("result"))


# ---------------------------------------------------------------- Claude

_CLAUDE_LINE = re.compile(
    r"^\s*Current\s+([^:]+?):\s*(\d{1,3})%\s*used(?:\s*[·•|\-]\s*resets?\s+(.+?))?\s*$",
    re.IGNORECASE,
)
_MONTHS = {
    name: index
    for index, name in enumerate(
        [
            "jan",
            "feb",
            "mar",
            "apr",
            "may",
            "jun",
            "jul",
            "aug",
            "sep",
            "oct",
            "nov",
            "dec",
        ],
        start=1,
    )
}


def _timezone(name):
    try:
        from zoneinfo import ZoneInfo

        return ZoneInfo(name)
    except Exception:  # missing tzdata or an unknown name
        return datetime.now().astimezone().tzinfo


def parse_claude_reset(text: str | None, now: datetime | None = None) -> int | None:
    """Turn ``Oct 15, 11:49pm (Asia/Kolkata)`` into a Unix timestamp, or None."""
    if not text:
        return None
    zone_match = re.search(r"\(([^)]+)\)", text)
    zone = _timezone(zone_match[1].strip()) if zone_match else None
    local = (
        (now.astimezone(zone) if now else datetime.now(zone))
        if zone
        else (now or datetime.now().astimezone())
    )
    time_match = re.search(r"(\d{1,2})(?::(\d{2}))?\s*([ap])\.?m", text, re.IGNORECASE)
    if not time_match:
        return None
    hour = int(time_match[1]) % 12 + (12 if time_match[3].lower() == "p" else 0)
    minute = int(time_match[2] or 0)
    date_match = re.search(r"([A-Za-z]{3})[a-z]*\.?\s+(\d{1,2})", text)
    try:
        if date_match and date_match[1].lower() in _MONTHS:
            month, day = _MONTHS[date_match[1].lower()], int(date_match[2])
            target = local.replace(
                month=month, day=day, hour=hour, minute=minute, second=0, microsecond=0
            )
            if target < local - timedelta(days=1):
                target = target.replace(year=local.year + 1)
        else:
            target = local.replace(hour=hour, minute=minute, second=0, microsecond=0)
            if target <= local:
                target += timedelta(days=1)
    except ValueError:
        return None
    return int(target.timestamp())


def parse_claude_usage(output: str, now: datetime | None = None) -> dict:
    windows = []
    for line in output.splitlines():
        match = _CLAUDE_LINE.match(line)
        if not match:
            continue
        name = match[1].strip().lower()
        used = int(match[2])
        resets = parse_claude_reset(match[3], now)
        if name == "session":
            window = _window("session", "Session", used, resets, 300)
        elif name.startswith("week"):
            qualifier = re.search(r"\(([^)]*)\)", name)
            detail = qualifier[1].strip() if qualifier else ""
            window = _window(
                "weekly" if detail in {"", "all models"} else f"weekly-{detail}",
                "Weekly" if detail in {"", "all models"} else f"Weekly · {detail}",
                used,
                resets,
                7 * 24 * 60,
            )
        else:
            window = _window(re.sub(r"\W+", "-", name), name.capitalize(), used, resets)
        windows.append(window)
    if not windows:
        raise ValueError("Claude Code did not report usage limits.")
    return {"plan": None, "windows": windows}


def _claude(binary: str) -> dict:
    result = subprocess.run(
        [*agents.launch_command(binary), "-p", "/usage"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=CLAUDE_TIMEOUT,
        stdin=subprocess.DEVNULL,
        cwd=os.environ.get("TEMP") or None,
        **({"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}),
    )
    return parse_claude_usage(result.stdout)


# ---------------------------------------------------------------- snapshot

_FETCHERS = {"codex": _codex, "claude": _claude}


def _provider_usage(provider: str, config: dict) -> dict:
    meta = agents.PROVIDERS[provider]
    entry = {
        "id": provider,
        "name": meta["name"],
        "available": False,
        "plan": None,
        "windows": [],
        "error": None,
    }
    if not config.get("enabled", True):
        entry["error"] = f"{meta['name']} is turned off."
        return entry
    binary = agents.resolve_binary(provider, config.get("binary", ""))
    if not binary:
        entry["error"] = f"{meta['name']} was not found."
        return entry
    try:
        entry.update(_FETCHERS[provider](binary), available=True)
    except subprocess.TimeoutExpired:
        entry["error"] = f"{meta['name']} took too long to report usage."
    except (OSError, subprocess.SubprocessError, ValueError, TimeoutError) as error:
        logging.info("Usage lookup for %s failed: %s", provider, error)
        entry["error"] = str(error) or f"{meta['name']} could not report usage."
    return entry


def collect(settings: dict) -> dict:
    results: dict[str, dict] = {}

    def run(provider):
        results[provider] = _provider_usage(
            provider, settings.get("providers", {}).get(provider, {})
        )

    threads = [
        threading.Thread(target=run, args=(provider,), daemon=True)
        for provider in PROVIDER_ORDER
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    return {
        "providers": [results[provider] for provider in PROVIDER_ORDER],
        "updated": time.time(),
    }


def snapshot(settings: dict, force: bool = False) -> dict:
    global _CACHE, _CACHED_AT
    with _LOCK:
        fresh = _CACHE is not None and time.monotonic() - _CACHED_AT < CACHE_SECONDS
        if fresh and not force:
            return _CACHE
        _CACHE = collect(settings)
        _CACHED_AT = time.monotonic()
        return _CACHE


def reset_cache():
    global _CACHE, _CACHED_AT
    with _LOCK:
        _CACHE, _CACHED_AT = None, 0.0
