from __future__ import annotations

import unittest

import codex_discord_approval_view as approval_view


class ApprovalViewExportTests(unittest.TestCase):
    def test_approval_view_module_exports_view_and_deps(self) -> None:
        self.assertTrue(hasattr(approval_view, "ApprovalView"))
        self.assertTrue(hasattr(approval_view, "ApprovalViewDeps"))
