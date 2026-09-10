from __future__ import annotations

import unittest

import codex_discord_bridge_protocols as bridge_protocols


class BridgeProtocolExportTests(unittest.TestCase):
    def test_bridge_protocol_module_exports_codex_bridge_shapes(self) -> None:
        self.assertTrue(hasattr(bridge_protocols, "CodexBridgeThreadLists"))
        self.assertTrue(hasattr(bridge_protocols, "CodexBridgeMirrorStatusModule"))
        self.assertTrue(hasattr(bridge_protocols, "CodexBridgeContextRefreshModule"))
