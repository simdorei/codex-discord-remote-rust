from __future__ import annotations

import unittest

import codex_discord_persistent_busy_choice_interaction as interaction_handler


class PersistentBusyChoiceInteractionExportTests(unittest.TestCase):
    def test_interaction_module_exports_handler_and_deps(self) -> None:
        self.assertTrue(hasattr(interaction_handler, "PersistentBusyChoiceInteractionDeps"))
        self.assertTrue(hasattr(interaction_handler, "handle_persistent_busy_choice_interaction"))
