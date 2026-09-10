from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import codex_bridge_state as bridge_state


class BridgeStateTests(unittest.TestCase):
    def test_load_json_rejects_non_object_state_with_typed_error(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"
            _ = state_path.write_text("[]", encoding="utf-8")

            with self.assertRaisesRegex(
                bridge_state.BridgeStateFormatError,
                "did not contain a JSON object",
            ):
                _ = bridge_state.load_json(state_path)

    def test_load_json_accepts_utf8_bom_state(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"
            _ = state_path.write_text('\ufeff{"selected_thread_id": "thread-1"}', encoding="utf-8")

            self.assertEqual({"selected_thread_id": "thread-1"}, bridge_state.load_json(state_path))

    def test_load_bridge_state_repairs_corrupt_json_with_backup(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"
            _ = state_path.write_bytes(b"\0" * 8)

            self.assertEqual({}, bridge_state.load_bridge_state(state_path))

            backups = list(state_path.parent.glob("state.json.corrupt-*.bak"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_bytes(), b"\0" * 8)
            self.assertEqual({}, bridge_state.load_json(state_path))

    def test_selected_thread_round_trips_and_clears(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"

            bridge_state.set_selected_thread_id(state_path, "thread-1")
            self.assertEqual(bridge_state.get_selected_thread_id(state_path), "thread-1")

            bridge_state.set_selected_thread_id(state_path, None)
            self.assertIsNone(bridge_state.get_selected_thread_id(state_path))

    def test_thread_settings_round_trip(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"

            bridge_state.remember_thread_settings(
                state_path,
                "thread-1",
                model="gpt-5.5",
                reasoning="xhigh",
                speed="fast",
            )

            self.assertEqual(
                bridge_state.get_saved_thread_settings(state_path, "thread-1"),
                {"model": "gpt-5.5", "reasoning": "xhigh", "speed": "fast"},
            )

    def test_codex_app_package_version_update_detection_records_current_version(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"

            first = bridge_state.record_codex_app_package_version(state_path, "26.611.8604.0")
            second = bridge_state.record_codex_app_package_version(state_path, "26.611.8604.0")
            changed = bridge_state.record_codex_app_package_version(state_path, "26.612.1.0")

            self.assertEqual(first.previous_version, None)
            self.assertFalse(first.update_detected)
            self.assertEqual(second.previous_version, "26.611.8604.0")
            self.assertFalse(second.update_detected)
            self.assertEqual(changed.previous_version, "26.611.8604.0")
            self.assertTrue(changed.update_detected)
            self.assertEqual(changed.current_version, "26.612.1.0")

    def test_live_approval_cache_expires_and_clears(self) -> None:
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            state_path = Path(temp_dir) / "state.json"
            pending_request: bridge_state.JsonObject = {
                "thread_id": "thread-1",
                "request_id": "req-1",
                "request_kind": "commandExecution",
                "method": "shell",
                "item_id": "item-1",
                "reason": "needs shell",
                "owner_client_id": "client-1",
            }

            bridge_state.cache_live_approval_request(state_path, pending_request, now=lambda: 100.0)

            self.assertEqual(
                bridge_state.get_cached_live_approval_request(
                    state_path,
                    "thread-1",
                    max_age_sec=120.0,
                    now=lambda: 101.0,
                ),
                {
                    "thread_id": "thread-1",
                    "request_id": "req-1",
                    "request_kind": "commandExecution",
                    "method": "shell",
                    "item_id": "item-1",
                    "reason": "needs shell",
                    "owner_client_id": "client-1",
                },
            )
            self.assertIsNone(
                bridge_state.get_cached_live_approval_request(
                    state_path,
                    "thread-1",
                    max_age_sec=120.0,
                    now=lambda: 300.0,
                )
            )

            bridge_state.clear_cached_live_approval_request(state_path, "thread-1")
            self.assertIsNone(bridge_state.get_cached_live_approval_request(state_path, "thread-1"))
