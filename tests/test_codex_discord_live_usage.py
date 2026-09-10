from __future__ import annotations

import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path
from typing import final

from codex_app_server_transport_replies import JsonMapping, JsonObject
from codex_discord_weekly_usage import build_weekly_usage_message
from tests.test_codex_discord_weekly_usage import FakeWeeklyUsageBridge


@final
class FakeUsageTransport:
    def __init__(
        self,
        responses: dict[str, JsonObject],
        *,
        error: Exception | None = None,
    ) -> None:
        self.responses = responses
        self.error = error
        self.requests: list[str] = []
        self.closed = False

    def request(
        self,
        method: str,
        params: JsonMapping | None = None,
        *,
        timeout_sec: float = 10.0,
    ) -> JsonObject:
        _ = (params, timeout_sec)
        self.requests.append(method)
        if self.error is not None:
            raise self.error
        return self.responses[method]

    def close(self) -> None:
        self.closed = True


class LiveUsageTests(unittest.TestCase):
    def test_uses_live_rate_limits_and_requested_daily_range(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            bridge = FakeWeeklyUsageBridge(Path(temp_dir))
            transport = FakeUsageTransport(
                {
                    "account/rateLimits/read": {
                        "rateLimits": {
                            "planType": "pro",
                            "limitId": "codex",
                            "primary": {
                                "usedPercent": 25,
                                "windowDurationMins": 10080,
                                "resetsAt": 1785813767,
                            },
                            "secondary": None,
                            "credits": {
                                "balance": "12",
                                "hasCredits": True,
                                "unlimited": False,
                            },
                            "rateLimitReachedType": None,
                        }
                    },
                    "account/usage/read": {
                        "dailyUsageBuckets": [
                            {"startDate": "2026-07-25", "tokens": 9000},
                            {"startDate": "2026-07-26", "tokens": 1000},
                            {"startDate": "2026-07-27", "tokens": 2000},
                            {"startDate": "2026-07-28", "tokens": 3000},
                        ],
                        "summary": {
                            "currentStreakDays": 3,
                            "longestStreakDays": 8,
                            "lifetimeTokens": 12000,
                            "peakDailyTokens": 4000,
                            "longestRunningTurnSec": 90,
                        },
                    },
                }
            )
            output = build_weekly_usage_message(
                3,
                bridge_module=bridge,
                format_percent_func=lambda value: f"{value}%",
                transport_factory=lambda: transport,
                now_func=lambda: datetime(2026, 7, 28, 12, 0, tzinfo=timezone.utc),
            )

        self.assertIn("Codex usage (3d live)", output)
        self.assertIn("plan: pro", output)
        self.assertIn("limit_id: codex", output)
        self.assertIn("primary: used=25% window=7d", output)
        self.assertIn("credits: balance=12 has_credits=yes unlimited=no", output)
        self.assertIn("2026-07-26: 1.0k", output)
        self.assertIn("2026-07-27: 2.0k", output)
        self.assertIn("2026-07-28: 3.0k", output)
        self.assertNotIn("2026-07-25: 9.0k", output)
        self.assertIn("total_tokens: 6.0k", output)
        self.assertIn("lifetime_tokens: 12.0k", output)
        self.assertEqual(
            transport.requests,
            ["account/rateLimits/read", "account/usage/read"],
        )
        self.assertTrue(transport.closed)

    def test_surfaces_live_failure_without_local_fallback(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            bridge = FakeWeeklyUsageBridge(Path(temp_dir))
            transport = FakeUsageTransport({}, error=RuntimeError("live query failed"))

            output = build_weekly_usage_message(
                7,
                bridge_module=bridge,
                format_percent_func=lambda value: f"{value}%",
                transport_factory=lambda: transport,
            )

        self.assertEqual(
            output,
            "Live usage unavailable: RuntimeError: live query failed",
        )
        self.assertNotIn("local", output.lower())
        self.assertTrue(transport.closed)


if __name__ == "__main__":
    _ = unittest.main()
