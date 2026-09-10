from __future__ import annotations

import unittest

import codex_discord_context_refresh_limits as refresh_limits


class ContextRefreshLimitsTests(unittest.TestCase):
    def test_clamp_context_refresh_limit_uses_default_for_blank_value(self) -> None:
        self.assertEqual(
            refresh_limits.clamp_context_refresh_limit(
                "",
                default=5,
                minimum=1,
                maximum=30,
            ),
            5,
        )

    def test_clamp_context_refresh_limit_clamps_to_bounds(self) -> None:
        self.assertEqual(
            refresh_limits.clamp_context_refresh_limit(
                "0",
                default=5,
                minimum=1,
                maximum=30,
            ),
            1,
        )
        self.assertEqual(
            refresh_limits.clamp_context_refresh_limit(
                "99",
                default=5,
                minimum=1,
                maximum=30,
            ),
            30,
        )

    def test_clamp_context_refresh_limit_uses_default_for_invalid_value(self) -> None:
        self.assertEqual(
            refresh_limits.clamp_context_refresh_limit(
                "invalid",
                default=5,
                minimum=1,
                maximum=30,
            ),
            5,
        )


if __name__ == "__main__":
    _ = unittest.main()
