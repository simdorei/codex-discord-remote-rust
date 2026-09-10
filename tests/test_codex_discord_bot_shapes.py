from __future__ import annotations

import unittest

import codex_discord_bot_shapes as bot_shapes


class BotShapeExportTests(unittest.TestCase):
    def test_bot_shape_module_exports_busy_choice_shapes(self) -> None:
        self.assertTrue(hasattr(bot_shapes, "BusyChoiceSourceMessage"))
        self.assertTrue(hasattr(bot_shapes, "RuntimeBusyChoiceAuthor"))
        self.assertTrue(hasattr(bot_shapes, "SlashAskSourceMessage"))
