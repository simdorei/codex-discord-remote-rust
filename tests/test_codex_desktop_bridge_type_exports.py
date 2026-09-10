from __future__ import annotations

import unittest
from pathlib import Path
from types import ModuleType
from typing import cast

import codex_desktop_bridge as bridge
import codex_desktop_bridge_archive_retry as archive_retry
import codex_desktop_bridge_impl as bridge_impl
import codex_desktop_bridge_impl_chunk04 as chunk04
import codex_desktop_bridge_impl_chunk06 as chunk06
import codex_desktop_bridge_impl_chunk07 as chunk07
import codex_desktop_bridge_ipc_pipe as ipc_pipe
import codex_desktop_bridge_sidecar_resolver as sidecar_resolver
from codex_bridge_state import JsonObject
from codex_thread_models import ThreadInfo


ROOT = Path(__file__).resolve().parents[1]


EXPECTED_EXPORTS = {
    "ThreadInfo",
    "_read_ipc_response",
    "_request_start_turn_via_ipc",
    "_write_ipc_message",
    "activate_thread_by_sidebar_v2",
    "archive_thread_once",
    "build_parser",
    "command_approval_reply",
    "command_archive",
    "command_ask",
    "command_delete_archive",
    "command_open",
    "command_stop",
    "command_tail",
    "command_use",
    "detect_running_codex_app_server_executable",
    "make_cli_handlers",
    "print_thread_list",
    "stop_codex_archive_lock_candidates",
    "verify_active_thread",
    "verify_active_thread_by_header",
    "wait_for_thread_activation",
}

def _runtime_attribute(module: ModuleType, name: str) -> object:
    return cast(object, getattr(module, name))


class DesktopBridgeTypeExportsTests(unittest.TestCase):
    def test_declared_export_ownership_is_unique(self) -> None:
        owners: dict[str, str] = {}
        for module in bridge_impl.BRIDGE_MODULES:
            declared = cast(tuple[str, ...], module.__all__)
            for name in declared:
                if name in bridge_impl.IGNORED_DECLARED_EXPORTS:
                    continue
                self.assertNotIn(name, owners, f"{name} is exported by two bridge modules")
                owners[name] = module.__name__

        self.assertGreater(len(owners), 300)

    def test_duplicate_declared_export_fails_composition(self) -> None:
        first = ModuleType("first")
        first.__all__ = ("shared",)
        first.shared = object()
        second = ModuleType("second")
        second.__all__ = ("shared",)
        second.shared = object()

        with self.assertRaises(bridge_impl.BridgeExportCollisionError):
            bridge_impl.compose_bridge_exports((first, second))

    def test_composition_uses_declared_exports_not_dir_scanning(self) -> None:
        source = (ROOT / "codex_desktop_bridge_impl.py").read_text(encoding="utf-8")

        self.assertNotIn("dir(module)", source)

    def test_missing_static_exports_exist_at_runtime(self) -> None:
        self.assertEqual([name for name in sorted(EXPECTED_EXPORTS) if not hasattr(bridge, name)], [])

    def test_representative_exports_keep_runtime_identity(self) -> None:
        self.assertIs(bridge.ThreadInfo, ThreadInfo)
        self.assertIs(bridge.archive_thread_once, archive_retry.archive_thread_once)
        self.assertIs(
            bridge.detect_running_codex_app_server_executable,
            sidecar_resolver.detect_running_codex_app_server_executable,
        )
        self.assertIs(_runtime_attribute(bridge, "_write_ipc_message"), ipc_pipe.write_ipc_message)
        self.assertIs(bridge.verify_active_thread, chunk04.verify_active_thread)
        self.assertIs(bridge.command_ask, chunk06.command_ask)

    def test_public_wrapper_assignment_propagates_and_restores(self) -> None:
        original = bridge.verify_active_thread

        def fake_verify_active_thread(thread_id: str) -> str | None:
            return f"fake:{thread_id}"

        try:
            bridge.verify_active_thread = fake_verify_active_thread
            self.assertIs(_runtime_attribute(bridge_impl, "verify_active_thread"), fake_verify_active_thread)
            self.assertIs(chunk04.verify_active_thread, fake_verify_active_thread)
        finally:
            bridge.verify_active_thread = original

        self.assertIs(_runtime_attribute(bridge_impl, "verify_active_thread"), original)
        self.assertIs(chunk04.verify_active_thread, original)

    def test_private_ipc_assignment_propagates_and_restores(self) -> None:
        original = _runtime_attribute(bridge, "_write_ipc_message")
        calls: list[tuple[int, JsonObject]] = []

        def fake_write_ipc_message(handle: int, payload: JsonObject) -> None:
            calls.append((handle, payload))

        try:
            setattr(bridge, "_write_ipc_message", fake_write_ipc_message)
            self.assertIs(_runtime_attribute(bridge_impl, "_write_ipc_message"), fake_write_ipc_message)
            self.assertIs(_runtime_attribute(chunk07, "_write_ipc_message"), fake_write_ipc_message)
            writer = _runtime_attribute(bridge, "_write_ipc_message")
            self.assertTrue(callable(writer))
            if not callable(writer):
                self.fail("_write_ipc_message is not callable")
            _ = writer(7, {"method": "test"})
            self.assertEqual(calls, [(7, {"method": "test"})])
        finally:
            setattr(bridge, "_write_ipc_message", original)

        self.assertIs(_runtime_attribute(bridge_impl, "_write_ipc_message"), original)
        self.assertIs(_runtime_attribute(chunk07, "_write_ipc_message"), original)


if __name__ == "__main__":
    _ = unittest.main()
