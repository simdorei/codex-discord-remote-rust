from __future__ import annotations

import unittest

import codex_discord_approval_button_action as approval_action


class ApprovalButtonActionExportTests(unittest.TestCase):
    def test_approval_action_module_exports_handler_and_deps(self) -> None:
        self.assertTrue(hasattr(approval_action, "ApprovalButtonActionDeps"))
        self.assertTrue(hasattr(approval_action, "handle_approval_button_submit"))
