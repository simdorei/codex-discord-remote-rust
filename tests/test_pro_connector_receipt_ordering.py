from __future__ import annotations

import importlib.util
import json
import os
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import Protocol, runtime_checkable
from unittest import mock

import codex_pro_connector_evidence as connector_evidence
import codex_pro_connector_transcript as connector_transcript


HOOK_PATH = Path(
    "plugins/codex-discord-remote/hooks/pro_connector_evidence_hook.py"
).resolve()
PLUGIN_ROOT = HOOK_PATH.parent.parent
SESSION_ID = "session-a"
TURN_ID = "turn-a"


@runtime_checkable
class HookModule(Protocol):
    PROTOCOL: str

    def canonical_probe_code(self, plugin_root: Path | None = None) -> str: ...

    def canonical_retry_probe_code(
        self, plugin_root: Path | None = None
    ) -> str: ...

    def process_post_tool_use(
        self,
        payload: Mapping[str, connector_transcript.JsonValue],
        plugin_data: Path | None = None,
        plugin_root: Path | None = None,
    ) -> bool: ...

    def _write_receipt(
        self,
        receipt: Mapping[str, connector_transcript.JsonValue],
        plugin_data: Path | None,
    ) -> bool: ...


class HookLoadError(RuntimeError):
    pass


def _load_hook() -> HookModule:
    spec = importlib.util.spec_from_file_location("receipt_ordering_hook", HOOK_PATH)
    if spec is None or spec.loader is None:
        raise HookLoadError("connector evidence hook could not be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if not isinstance(module, HookModule):
        raise HookLoadError("connector evidence hook contract did not match")
    return module


def _evidence(hook: HookModule, *, verified: bool) -> connector_transcript.JsonObject:
    if verified:
        return {
            "protocol": hook.PROTOCOL,
            "browser_type": "chrome",
            "status": "verified",
            "connector_name": connector_evidence.CONNECTOR_NAME,
            "connector_path": connector_evidence.CONNECTOR_PATH,
            "chat_mode": "chat",
            "pro_mode": True,
            "action": "attached",
        }
    return {
        "protocol": hook.PROTOCOL,
        "browser_type": "chrome",
        "status": "failed",
        "connector_name": connector_evidence.CONNECTOR_NAME,
        "connector_path": connector_evidence.CONNECTOR_PATH,
        "chat_mode": "unverified",
        "pro_mode": False,
        "action": "none",
        "failed_stage": "connector_search",
    }


def _hook_payload(
    hook: HookModule, *, retry: bool, response: str
) -> Mapping[str, connector_transcript.JsonValue]:
    return {
        "hook_event_name": "PostToolUse",
        "session_id": SESSION_ID,
        "turn_id": TURN_ID,
        "tool_name": "functions.exec",
        "tool_input": (
            hook.canonical_retry_probe_code() if retry else hook.canonical_probe_code()
        ),
        "tool_response": response,
    }


def _record(payload: connector_transcript.JsonObject) -> str:
    return json.dumps({"type": "response_item", "payload": payload})


def _write_transcript(
    codex_home: Path,
    hook: HookModule,
    attempts: tuple[tuple[bool, str], ...],
) -> None:
    transcript = (
        codex_home
        / "sessions/2026/08/25"
        / f"rollout-2026-08-25T00-00-00-{SESSION_ID}.jsonl"
    )
    transcript.parent.mkdir(parents=True)
    metadata = {"turn_id": TURN_ID}
    records: list[str] = []
    for index, (retry, response) in enumerate(attempts):
        call_id = f"call-{index}"
        code = (
            hook.canonical_retry_probe_code(PLUGIN_ROOT)
            if retry
            else hook.canonical_probe_code(PLUGIN_ROOT)
        )
        records.append(
            _record(
                {
                    "type": "custom_tool_call",
                    "name": "functions.exec",
                    "input": code,
                    "call_id": call_id,
                    "internal_chat_message_metadata_passthrough": metadata,
                }
            )
        )
        records.append(
            _record(
                {
                    "type": "custom_tool_call_output",
                    "call_id": call_id,
                    "output": response,
                    "internal_chat_message_metadata_passthrough": metadata,
                }
            )
        )
    transcript.write_text("\n".join(records) + "\n", encoding="utf-8")


class ProConnectorReceiptOrderingTests(unittest.TestCase):
    def test_later_invalid_retry_rejects_stale_receipt_when_write_fails(self) -> None:
        hook = _load_hook()
        verified = json.dumps(_evidence(hook, verified=True))
        primary = _hook_payload(hook, retry=False, response=verified)
        retry = _hook_payload(hook, retry=True, response="not evidence")

        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            plugin_data = Path(raw_data)
            self.assertTrue(hook.process_post_tool_use(primary, plugin_data))
            _write_transcript(
                Path(raw_home),
                hook,
                ((False, verified), (True, "not evidence")),
            )

            with mock.patch.object(hook, "_write_receipt", return_value=False):
                self.assertFalse(hook.process_post_tool_use(retry, plugin_data))

            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    SESSION_ID, TURN_ID, plugin_data=plugin_data
                )

    def test_delayed_primary_callback_cannot_override_failed_retry(self) -> None:
        hook = _load_hook()
        verified = json.dumps(_evidence(hook, verified=True))
        failed = json.dumps(_evidence(hook, verified=False))
        primary = _hook_payload(hook, retry=False, response=verified)
        retry = _hook_payload(hook, retry=True, response=failed)

        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            plugin_data = Path(raw_data)
            _write_transcript(
                Path(raw_home), hook, ((False, verified), (True, failed))
            )
            self.assertTrue(hook.process_post_tool_use(primary, plugin_data))
            self.assertTrue(hook.process_post_tool_use(retry, plugin_data))
            self.assertTrue(hook.process_post_tool_use(primary, plugin_data))

            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    SESSION_ID, TURN_ID, plugin_data=plugin_data
                )

    def test_invalid_retry_receipt_has_complete_failed_shape(self) -> None:
        hook = _load_hook()
        primary = _hook_payload(
            hook,
            retry=False,
            response=json.dumps(_evidence(hook, verified=True)),
        )
        retry = _hook_payload(hook, retry=True, response="not evidence")

        with tempfile.TemporaryDirectory() as raw_data:
            plugin_data = Path(raw_data)
            self.assertTrue(hook.process_post_tool_use(primary, plugin_data))
            self.assertTrue(hook.process_post_tool_use(retry, plugin_data))
            receipt = json.loads(
                connector_evidence._receipt_path(
                    SESSION_ID, TURN_ID, plugin_data
                ).read_text(encoding="utf-8")
            )

            self.assertEqual(
                receipt,
                {
                    "protocol": hook.PROTOCOL,
                    "browser_type": "chrome",
                    "status": "failed",
                    "connector_name": connector_evidence.CONNECTOR_NAME,
                    "connector_path": connector_evidence.CONNECTOR_PATH,
                    "chat_mode": "unverified",
                    "pro_mode": False,
                    "action": "none",
                    "failed_stage": "evidence_invalid",
                    "session_id": SESSION_ID,
                    "turn_id": TURN_ID,
                    "probe_sha256": connector_evidence.EXPECTED_PROBE_SHA256,
                },
            )


if __name__ == "__main__":
    _ = unittest.main()
