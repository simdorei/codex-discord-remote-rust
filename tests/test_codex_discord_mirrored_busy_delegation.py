from __future__ import annotations

import unittest

import codex_discord_mirrored_busy_delegation as mirrored_busy_delegation


class MirroredBusyDelegationExportTests(unittest.TestCase):
    def test_mirrored_busy_delegation_module_exports_helper_and_deps(self) -> None:
        self.assertTrue(hasattr(mirrored_busy_delegation, "MirroredBusyDelegationDeps"))
        self.assertTrue(hasattr(mirrored_busy_delegation, "wait_for_mirrored_busy_delegation_settle"))
