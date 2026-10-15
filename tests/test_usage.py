"""Plan usage limits: parse what the CLIs print, and degrade quietly when they can't."""

from datetime import datetime, timedelta, timezone
import unittest
from unittest.mock import patch

import usage

CLAUDE_OUTPUT = """You are currently using your subscription to power your Claude Code usage

Current session: 93% used · resets Oct 15, 11:49pm (Asia/Kolkata)
Current week (all models): 38% used · resets Oct 19, 4:29am (Asia/Kolkata)

What's contributing to your limits usage?
Approximate, based on local sessions on this machine.

Last 24h · 585 requests · 9 sessions
  81% of your usage was at >150k context
"""

IST = timezone(timedelta(hours=5, minutes=30))


class ClaudeUsageTests(unittest.TestCase):
    def test_named_reset_zones_are_independent_of_the_host_timezone(self):
        now = datetime(2026, 7, 15, 12, 0, tzinfo=timezone.utc)
        for zone, expected in (
            ("Asia/Kolkata", datetime(2026, 7, 15, 18, 19, tzinfo=timezone.utc)),
            ("America/New_York", datetime(2026, 7, 16, 3, 49, tzinfo=timezone.utc)),
            ("Europe/London", datetime(2026, 7, 15, 22, 49, tzinfo=timezone.utc)),
        ):
            with self.subTest(zone=zone):
                self.assertEqual(
                    usage.parse_claude_reset(f"Jul 15, 11:49pm ({zone})", now),
                    int(expected.timestamp()),
                )

    def test_session_and_weekly_windows_are_read(self):
        now = datetime(2026, 10, 15, 19, 0, tzinfo=IST)
        state = usage.parse_claude_usage(CLAUDE_OUTPUT, now)
        session, weekly = state["windows"]
        self.assertEqual(
            (session["id"], session["label"], session["used_percent"]),
            ("session", "Session", 93),
        )
        self.assertEqual(session["left_percent"], 7)
        self.assertEqual((weekly["label"], weekly["left_percent"]), ("Weekly", 62))
        for window in (session, weekly):
            self.assertIsInstance(window["resets_at"], int)
        # The reset time is read in the CLI's own time zone.
        reset = datetime.fromtimestamp(session["resets_at"], IST)
        self.assertEqual(
            (reset.month, reset.day, reset.hour, reset.minute), (10, 15, 23, 49)
        )
        self.assertGreater(weekly["resets_at"], session["resets_at"])

    def test_other_weekly_windows_and_missing_reset_are_tolerated(self):
        text = (
            "Current week (Opus): 12% used\n"
            "Current session: 0% used · resets 4:05am (Asia/Kolkata)\n"
            "Current week (all models): 100% used · resets Jan 2, 9:00am (Asia/Kolkata)\n"
        )
        state = usage.parse_claude_usage(
            text, datetime(2026, 12, 30, 12, 0, tzinfo=IST)
        )
        opus, session, weekly = state["windows"]
        self.assertEqual((opus["label"], opus["resets_at"]), ("Weekly · opus", None))
        self.assertEqual(session["left_percent"], 100)
        # A date earlier in the calendar than today means next year.
        reset = datetime.fromtimestamp(weekly["resets_at"], IST)
        self.assertEqual((reset.year, reset.month, reset.day), (2027, 1, 2))
        self.assertEqual(weekly["left_percent"], 0)

    def test_unrecognised_output_is_an_error_not_zero_percent(self):
        for text in ("", "Unknown command: /usage", "Current session: n/a"):
            with self.subTest(text=text), self.assertRaises(ValueError):
                usage.parse_claude_usage(text)


class CodexUsageTests(unittest.TestCase):
    def test_weekly_window_and_plan_are_read(self):
        state = usage.parse_codex_limits(
            {
                "rateLimits": {
                    "planType": "plus",
                    "primary": {
                        "usedPercent": 30,
                        "windowDurationMins": 10080,
                        "resetsAt": 1792652259,
                    },
                    "secondary": None,
                }
            }
        )
        self.assertEqual(state["plan"], "plus")
        (weekly,) = state["windows"]
        self.assertEqual(
            (weekly["label"], weekly["left_percent"], weekly["resets_at"]),
            ("Weekly", 70, 1792652259),
        )

    def test_short_and_long_windows_are_labelled(self):
        state = usage.parse_codex_limits(
            {
                "rateLimits": {
                    "primary": {"usedPercent": 10, "windowDurationMins": 300},
                    "secondary": {"usedPercent": 55, "windowDurationMins": 10080},
                    "planType": "unknown",
                }
            }
        )
        self.assertIsNone(state["plan"])
        self.assertEqual([w["label"] for w in state["windows"]], ["Session", "Weekly"])

    def test_missing_limits_raise(self):
        for result in (None, {}, {"rateLimits": None}, {"rateLimits": {}}):
            with self.subTest(result=result), self.assertRaises(ValueError):
                usage.parse_codex_limits(result)

    def test_invalid_usage_is_unavailable_instead_of_a_misleading_number(self):
        for used in (None, "unavailable", True, float("nan"), float("inf")):
            with self.subTest(used=used), self.assertRaises(ValueError):
                usage.parse_codex_limits(
                    {"rateLimits": {"primary": {"usedPercent": used}}}
                )


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        usage.reset_cache()
        self.addCleanup(usage.reset_cache)

    def test_providers_report_independently_and_failures_are_contained(self):
        def codex(binary):
            return {"plan": "plus", "windows": [usage._window("w", "Weekly", 10)]}

        def claude(binary):
            raise ValueError("Claude Code did not report usage limits.")

        settings = {
            "providers": {
                "codex": {"enabled": True, "binary": ""},
                "claude": {"enabled": True, "binary": ""},
            }
        }
        with (
            patch.object(usage.agents, "resolve_binary", return_value="cli"),
            patch.dict(usage._FETCHERS, {"codex": codex, "claude": claude}),
        ):
            state = usage.snapshot(settings)
        codex_state, claude_state = state["providers"]
        self.assertTrue(codex_state["available"])
        self.assertEqual(codex_state["windows"][0]["left_percent"], 90)
        self.assertFalse(claude_state["available"])
        self.assertIn("did not report", claude_state["error"])

    def test_disabled_or_missing_cli_is_explained_and_results_are_cached(self):
        calls = []
        settings = {
            "providers": {
                "codex": {"enabled": False, "binary": ""},
                "claude": {"enabled": True, "binary": ""},
            }
        }
        with patch.object(
            usage.agents, "resolve_binary", side_effect=lambda *a: calls.append(a)
        ):
            first = usage.snapshot(settings)
            second = usage.snapshot(settings)
            forced = usage.snapshot(settings, force=True)
        self.assertIs(first, second)
        self.assertIsNot(first, forced)
        codex_state, claude_state = first["providers"]
        self.assertIn("turned off", codex_state["error"])
        self.assertIn("was not found", claude_state["error"])
        self.assertEqual(len(calls), 2)  # one lookup per snapshot, not per request


if __name__ == "__main__":
    unittest.main()
