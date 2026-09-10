from __future__ import annotations

import unittest

import codex_discord_prompt_delivery_snapshot as prompt_delivery_snapshot


class PromptDeliverySnapshotExportTests(unittest.TestCase):
    def test_prompt_delivery_snapshot_module_exports_helper_and_deps(self) -> None:
        self.assertTrue(hasattr(prompt_delivery_snapshot, "PromptDeliverySnapshotDeps"))
        self.assertTrue(hasattr(prompt_delivery_snapshot, "snapshot_ask_prompt_delivery_state"))
