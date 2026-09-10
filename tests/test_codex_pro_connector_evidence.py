from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from typing import Protocol, cast
from unittest import mock

import codex_pro_connector_evidence as connector_evidence
import codex_pro_connector_transcript as connector_transcript


HOOK_PATH = Path(
    "plugins/codex-discord-remote/hooks/pro_connector_evidence_hook.py"
).resolve()
PLUGIN_ROOT = HOOK_PATH.parent.parent


class HookModule(Protocol):
    def canonical_probe_code(self, plugin_root: Path | None = None) -> str: ...

    def canonical_retry_probe_code(
        self, plugin_root: Path | None = None
    ) -> str: ...


class HookLoadError(RuntimeError):
    pass


def _load_hook() -> HookModule:
    spec = importlib.util.spec_from_file_location("connector_evidence_hook", HOOK_PATH)
    if spec is None or spec.loader is None:
        raise HookLoadError("connector evidence hook could not be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return cast(HookModule, module)


def _evidence(status: str = "verified") -> connector_transcript.JsonObject:
    return {
        "protocol": connector_evidence.PROTOCOL,
        "browser_type": "chrome",
        "status": status,
        "connector_name": connector_evidence.CONNECTOR_NAME,
        "connector_path": connector_evidence.CONNECTOR_PATH,
        "chat_mode": "chat" if status == "verified" else "unverified",
        "pro_mode": status == "verified",
        "action": "attached" if status == "verified" else "none",
        **({} if status == "verified" else {"failed_stage": "connector_search"}),
    }


def _turn_record(payload: connector_transcript.JsonObject) -> str:
    return json.dumps(
        {
            "type": "response_item",
            "payload": payload,
        }
    )


def _write_transcript(
    codex_home: Path,
    *,
    calls: tuple[tuple[str, connector_transcript.JsonObject | None], ...],
    retry_call_ids: frozenset[str] = frozenset(),
) -> None:
    transcript = (
        codex_home
        / "sessions/2026/08/24"
        / "rollout-2026-08-24T00-00-00-session-a.jsonl"
    )
    transcript.parent.mkdir(parents=True)
    hook = _load_hook()
    records: list[str] = []
    for call_id, evidence in calls:
        code = (
            hook.canonical_retry_probe_code(PLUGIN_ROOT)
            if call_id in retry_call_ids
            else hook.canonical_probe_code(PLUGIN_ROOT)
        )
        metadata = {"turn_id": "turn-a"}
        records.append(
            _turn_record(
                {
                    "type": "custom_tool_call",
                    "name": "functions.exec",
                    "input": code,
                    "call_id": call_id,
                    "internal_chat_message_metadata_passthrough": metadata,
                }
            )
        )
        if evidence is not None:
            records.append(
                _turn_record(
                    {
                        "type": "custom_tool_call_output",
                        "call_id": call_id,
                        "output": json.dumps(evidence),
                        "internal_chat_message_metadata_passthrough": metadata,
                    }
                )
            )
    transcript.write_text("\n".join(records) + "\n", encoding="utf-8")


class ProConnectorEvidenceTests(unittest.TestCase):
    def test_exact_verified_receipt_is_required(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            plugin_data = Path(raw_dir)
            key = hashlib.sha256(b"session-a\0turn-a").hexdigest()
            receipt_path = plugin_data / "pro-connector-evidence" / f"{key}.json"
            receipt_path.parent.mkdir(parents=True)
            receipt_path.write_text(
                json.dumps(
                    {
                        "protocol": connector_evidence.PROTOCOL,
                        "session_id": "session-a",
                        "turn_id": "turn-a",
                        "browser_type": "chrome",
                        "status": "verified",
                        "connector_name": connector_evidence.CONNECTOR_NAME,
                        "connector_path": connector_evidence.CONNECTOR_PATH,
                        "chat_mode": "chat",
                        "pro_mode": True,
                        "action": "attached",
                        "probe_sha256": connector_evidence.EXPECTED_PROBE_SHA256,
                    }
                ),
                encoding="utf-8",
            )

            connector_evidence.require_verified_evidence(
                "session-a", "turn-a", plugin_data=plugin_data
            )

            with self.assertRaisesRegex(
                connector_evidence.ProConnectorUnavailableError,
                "pro_connector_unavailable",
            ):
                connector_evidence.require_verified_evidence(
                    "session-a", "turn-b", plugin_data=plugin_data
                )

    def test_exact_verified_transcript_is_used_when_receipt_is_absent(self) -> None:
        # Given: the trusted helper completed in the exact turn without a receipt file.
        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            _write_transcript(
                Path(raw_home),
                calls=(("call-a", _evidence()),),
            )

            # When/Then: the public verifier accepts the exact transcript evidence.
            connector_evidence.require_verified_evidence(
                "session-a",
                "turn-a",
                plugin_data=Path(raw_data),
            )

    def test_corrupt_receipt_cannot_be_bypassed_by_verified_transcript(self) -> None:
        # Given: a valid transcript exists, but the exact receipt file is corrupt.
        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            plugin_data = Path(raw_data)
            _write_transcript(Path(raw_home), calls=(("call-a", _evidence()),))
            key = hashlib.sha256(b"session-a\0turn-a").hexdigest()
            receipt = plugin_data / "pro-connector-evidence" / f"{key}.json"
            receipt.parent.mkdir(parents=True)
            receipt.write_text("{", encoding="utf-8")

            # When/Then: corruption fails closed instead of using the fallback.
            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    "session-a",
                    "turn-a",
                    plugin_data=plugin_data,
                )

    def test_only_last_canonical_attempt_can_authorize_the_turn(self) -> None:
        # Given: a verified helper call is followed by a failed canonical retry.
        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            _write_transcript(
                Path(raw_home),
                calls=(
                    ("call-a", _evidence()),
                    ("call-b", _evidence("failed")),
                ),
                retry_call_ids=frozenset({"call-b"}),
            )

            # When/Then: the earlier success cannot override the later failure.
            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    "session-a",
                    "turn-a",
                    plugin_data=Path(raw_data),
                )

    def test_incomplete_last_canonical_attempt_fails_closed(self) -> None:
        # Given: a completed success is followed by a canonical call without output.
        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            _write_transcript(
                Path(raw_home),
                calls=(("call-a", _evidence()), ("call-b", None)),
                retry_call_ids=frozenset({"call-b"}),
            )

            # When/Then: the incomplete last attempt prevents authorization.
            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    "session-a",
                    "turn-a",
                    plugin_data=Path(raw_data),
                )

    def test_verified_retry_alias_supersedes_failed_primary_transcript(self) -> None:
        # Given: a failed primary attempt is followed by the exact retry wrapper.
        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            _write_transcript(
                Path(raw_home),
                calls=(
                    ("call-a", _evidence("failed")),
                    ("call-b", _evidence()),
                ),
                retry_call_ids=frozenset({"call-b"}),
            )

            # When/Then: the verified retry is the exact-turn authorization result.
            connector_evidence.require_verified_evidence(
                "session-a",
                "turn-a",
                plugin_data=Path(raw_data),
            )

    def test_transcript_evidence_is_scoped_to_exact_turn(self) -> None:
        # Given: a valid connector transcript belongs to another turn.
        with (
            tempfile.TemporaryDirectory() as raw_home,
            tempfile.TemporaryDirectory() as raw_data,
            mock.patch.dict(os.environ, {"CODEX_HOME": raw_home}),
        ):
            _write_transcript(Path(raw_home), calls=(("call-a", _evidence()),))

            # When/Then: the verifier cannot reuse it for the requested turn.
            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    "session-a",
                    "turn-b",
                    plugin_data=Path(raw_data),
                )


if __name__ == "__main__":
    _ = unittest.main()
