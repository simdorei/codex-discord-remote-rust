from __future__ import annotations

import unittest

import codex_discord_busy_choice_view as busy_choice_view


class BusyChoiceViewExportTests(unittest.TestCase):
    def test_busy_choice_view_module_exports_view_and_deps(self) -> None:
        self.assertTrue(hasattr(busy_choice_view, "BusyChoiceView"))
        self.assertTrue(hasattr(busy_choice_view, "BusyChoiceViewDeps"))
