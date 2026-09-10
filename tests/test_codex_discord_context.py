import unittest
from datetime import datetime, timezone
from unittest import mock

import codex_desktop_bridge as bridge
import codex_discord_context as context


class DiscordContextTests(unittest.TestCase):
    def test_weekly_usage_message_uses_real_bridge_numeric_helpers(self) -> None:
        transport = mock.Mock()
        transport.request.side_effect = [
            {
                "rateLimits": {
                    "primary": {
                        "usedPercent": 12.5,
                        "windowDurationMins": 60,
                        "resetsAt": 1893456000,
                    }
                }
            },
            {
                "dailyUsageBuckets": [
                    {"startDate": "2026-07-28", "tokens": 12345},
                ]
            },
        ]

        output = context.build_weekly_usage_message(
            7,
            bridge_module=bridge,
            format_percent_func=lambda value: f"{value}%",
            transport_factory=lambda: transport,
            now_func=lambda: datetime(2026, 7, 28, tzinfo=timezone.utc),
        )

        self.assertIn("Codex usage (7d live)", output)
        self.assertIn("primary: used=12.5% window=1h resets=2030-01-01", output)
        self.assertIn("2026-07-28: 12.3k", output)
        self.assertIn("total_tokens: 12.3k", output)
        transport.close.assert_called_once_with()


if __name__ == "__main__":
    _ = unittest.main()
