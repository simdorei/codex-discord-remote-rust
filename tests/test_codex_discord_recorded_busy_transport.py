from __future__ import annotations

import unittest

import codex_discord_recorded_busy_transport as recorded_busy_transport


class RecordedBusyTransportExportTests(unittest.TestCase):
    def test_recorded_busy_transport_module_exports_helper_and_deps(self) -> None:
        self.assertTrue(hasattr(recorded_busy_transport, "RecordedBusyTransportDeps"))
        self.assertTrue(hasattr(recorded_busy_transport, "handle_recorded_busy_transport_prompt"))
