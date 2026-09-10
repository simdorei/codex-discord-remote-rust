from __future__ import annotations

import unittest

import codex_discord_input_choice_view as input_choice_view


class InputChoiceViewExportTests(unittest.TestCase):
    def test_input_choice_view_module_exports_view_button_and_deps(self) -> None:
        self.assertTrue(hasattr(input_choice_view, "InputChoiceButton"))
        self.assertTrue(hasattr(input_choice_view, "InputChoiceView"))
        self.assertTrue(hasattr(input_choice_view, "InputChoiceViewDeps"))
