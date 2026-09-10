from __future__ import annotations

import unittest

import codex_discord_codex_app_menu as codex_app_menu


class CodexAppMenuExportTests(unittest.TestCase):
    def test_codex_app_menu_module_exports_helper_and_deps(self) -> None:
        self.assertTrue(hasattr(codex_app_menu, "CodexAppMenuDeps"))
        self.assertTrue(hasattr(codex_app_menu, "send_codex_app_menu_if_available"))
