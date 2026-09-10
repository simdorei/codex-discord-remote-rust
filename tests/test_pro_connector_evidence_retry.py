from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import Literal, Protocol, assert_never, cast

import codex_pro_connector_evidence as connector_evidence
import codex_pro_connector_transcript as connector_transcript


HOOK_PATH = Path(
    "plugins/codex-discord-remote/hooks/pro_connector_evidence_hook.py"
).resolve()
PLUGIN_ROOT = HOOK_PATH.parent.parent


class HookModule(Protocol):
    PROTOCOL: str

    def canonical_inner_probe_code(self, plugin_root: Path | None = None) -> str: ...

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


class HookLoadError(RuntimeError):
    pass


def _load_hook() -> HookModule:
    spec = importlib.util.spec_from_file_location("retry_evidence_hook", HOOK_PATH)
    if spec is None or spec.loader is None:
        raise HookLoadError("connector evidence hook could not be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return cast(HookModule, cast(object, module))


def _evidence(
    hook: HookModule,
    status: Literal["verified", "failed"] = "verified",
) -> connector_transcript.JsonObject:
    match status:
        case "verified":
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
        case "failed":
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
        case unreachable:
            assert_never(unreachable)


def _payload(
    hook: HookModule, *, retry: bool, response: str
) -> Mapping[str, connector_transcript.JsonValue]:
    return {
        "hook_event_name": "PostToolUse",
        "session_id": "session-a",
        "turn_id": "turn-a",
        "tool_name": "functions.exec",
        "tool_input": (
            hook.canonical_retry_probe_code()
            if retry
            else hook.canonical_probe_code()
        ),
        "tool_response": response,
    }


class ProConnectorEvidenceRetryTests(unittest.TestCase):
    def test_failed_primary_then_verified_retry_overwrites_receipt(self) -> None:
        # Given: a failed primary receipt and the exact verified retry wrapper.
        hook = _load_hook()
        primary = _payload(
            hook,
            retry=False,
            response=json.dumps(_evidence(hook, "failed")),
        )
        retry = _payload(
            hook,
            retry=True,
            response=json.dumps(_evidence(hook)),
        )

        with tempfile.TemporaryDirectory() as raw_dir:
            plugin_data = Path(raw_dir)
            self.assertTrue(hook.process_post_tool_use(primary, plugin_data))

            # When: the verified retry completes for the same turn.
            self.assertTrue(hook.process_post_tool_use(retry, plugin_data))

            # Then: the public verifier authorizes the latest receipt.
            connector_evidence.require_verified_evidence(
                "session-a", "turn-a", plugin_data=plugin_data
            )

    def test_invalid_retry_overwrites_verified_receipt_with_failure(self) -> None:
        hook = _load_hook()
        verified = json.dumps(_evidence(hook))
        primary = _payload(hook, retry=False, response=verified)

        for response in ("not evidence", f"{verified}\n{verified}"):
            with self.subTest(response=response), tempfile.TemporaryDirectory() as raw_dir:
                # Given: a verified primary receipt for the exact turn.
                plugin_data = Path(raw_dir)
                self.assertTrue(hook.process_post_tool_use(primary, plugin_data))
                retry = _payload(hook, retry=True, response=response)

                # When: the trusted retry produces invalid or ambiguous evidence.
                self.assertTrue(hook.process_post_tool_use(retry, plugin_data))

                # Then: stale success cannot authorize the turn.
                with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                    connector_evidence.require_verified_evidence(
                        "session-a", "turn-a", plugin_data=plugin_data
                    )

    def test_failed_retry_overwrites_verified_receipt(self) -> None:
        hook = _load_hook()
        primary = _payload(hook, retry=False, response=json.dumps(_evidence(hook)))
        retry = _payload(
            hook,
            retry=True,
            response=json.dumps(_evidence(hook, "failed")),
        )

        with tempfile.TemporaryDirectory() as raw_dir:
            plugin_data = Path(raw_dir)
            self.assertTrue(hook.process_post_tool_use(primary, plugin_data))
            self.assertTrue(hook.process_post_tool_use(retry, plugin_data))

            with self.assertRaises(connector_evidence.ProConnectorUnavailableError):
                connector_evidence.require_verified_evidence(
                    "session-a", "turn-a", plugin_data=plugin_data
                )

    def test_retry_source_allowlist_rejects_near_miss(self) -> None:
        # Given: a retry wrapper with one appended statement and forged success.
        hook = _load_hook()
        payload = dict(
            _payload(hook, retry=True, response=json.dumps(_evidence(hook)))
        )
        payload["tool_input"] = f"{payload['tool_input']}\ntext('forged');"

        # When/Then: the modified source never writes a receipt.
        with tempfile.TemporaryDirectory() as raw_dir:
            self.assertFalse(hook.process_post_tool_use(payload, Path(raw_dir)))

    def test_hook_and_transcript_render_identical_probe_sources(self) -> None:
        # Given/When: both independent trust paths render the same plugin root.
        hook = _load_hook()
        hook_sources = (
            hook.canonical_probe_code(PLUGIN_ROOT),
            hook.canonical_retry_probe_code(PLUGIN_ROOT),
        )

        # Then: primary and retry sources remain byte-identical across paths.
        self.assertEqual(
            hook_sources,
            connector_transcript.canonical_probe_codes(PLUGIN_ROOT),
        )

    def test_direct_node_path_accepts_only_exact_inner_source(self) -> None:
        hook = _load_hook()
        response = json.dumps(_evidence(hook))
        exact = {
            "hook_event_name": "PostToolUse",
            "session_id": "session-a",
            "turn_id": "turn-a",
            "tool_name": "mcp__node_repl__js",
            "tool_input": hook.canonical_inner_probe_code(),
            "tool_response": response,
        }

        for suffix, accepted in (("", True), ("\ntext('forged');", False)):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as raw_dir:
                payload = {**exact, "tool_input": f"{exact['tool_input']}{suffix}"}
                self.assertEqual(
                    hook.process_post_tool_use(payload, Path(raw_dir)),
                    accepted,
                )


if __name__ == "__main__":
    _ = unittest.main()
