from __future__ import annotations

import unittest

import codex_discord_input_choice_button_action as input_action


class InputChoiceButtonActionExportTests(unittest.TestCase):
    def test_input_action_module_exports_handler_and_deps(self) -> None:
        self.assertTrue(hasattr(input_action, "InputChoiceButtonActionDeps"))
        self.assertTrue(hasattr(input_action, "handle_input_choice_button_submit"))
