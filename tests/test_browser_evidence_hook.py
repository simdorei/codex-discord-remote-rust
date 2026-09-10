from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import ClassVar, Protocol, cast, override


HOOK_PATH = Path(
    "plugins/codex-discord-remote/hooks/browser_evidence_hook.py"
).resolve()


class HookModule(Protocol):
    PROTOCOL: str

    def canonical_probe_code(self, plugin_root: Path | None = None) -> str: ...

    def canonical_inner_probe_code(
        self, plugin_root: Path | None = None
    ) -> str: ...

    def process_post_tool_use(
        self,
        payload: Mapping[str, object],
        plugin_data: Path | None = None,
        plugin_root: Path | None = None,
    ) -> bool: ...

    def process_stop(
        self, payload: Mapping[str, object], plugin_data: Path | None = None
    ) -> dict[str, str] | None: ...


def _load_hook() -> HookModule:
    spec = importlib.util.spec_from_file_location("browser_evidence_hook", HOOK_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("browser evidence hook could not be loaded")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return cast(HookModule, cast(object, module))


def _post_payload(hook: HookModule, status: str = "unavailable") -> dict[str, object]:
    can_report = status == "unavailable"
    evidence: dict[str, object] = {
        "protocol": hook.PROTOCOL,
        "browser_type": "chrome",
        "status": status,
        "can_report_unavailable": can_report,
        "reason": "test evidence",
    }
    if status == "unavailable":
        evidence.update(
            failed_stage="select_chrome_retry",
            public_error="Error: browser offline",
        )
    return {
        "hook_event_name": "PostToolUse",
        "session_id": "session-a",
        "turn_id": "turn-a",
        "tool_name": "functions.exec",
        "tool_use_id": "tool-a",
        "tool_input": hook.canonical_probe_code(),
        "tool_response": f"Script completed\nOutput:\n{json.dumps(evidence)}",
    }


def _stop_payload(message: str, turn_id: str = "turn-a") -> dict[str, object]:
    return {
        "hook_event_name": "Stop",
        "session_id": "session-a",
        "turn_id": turn_id,
        "last_assistant_message": message,
        "stop_hook_active": False,
    }


class BrowserEvidenceHookTests(unittest.TestCase):
    hook: ClassVar[HookModule]

    @classmethod
    @override
    def setUpClass(cls) -> None:
        cls.hook = _load_hook()

    def test_exact_probe_records_same_turn_unavailable_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            data_dir = Path(raw_dir)
            recorded = self.hook.process_post_tool_use(
                _post_payload(self.hook), data_dir
            )

            self.assertTrue(recorded)
            output = self.hook.process_stop(
                _stop_payload("Chrome is unavailable."), data_dir
            )
            self.assertIsNone(output)

    def test_printed_probe_code_is_exact_and_legacy_newline_is_accepted(self) -> None:
        completed = subprocess.run(
            [
                str(Path(".python-portable/python.exe").resolve()),
                str(HOOK_PATH),
                "print-probe-code",
            ],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        self.assertEqual(completed.stdout, self.hook.canonical_probe_code())

        with tempfile.TemporaryDirectory() as raw_dir:
            payload = _post_payload(self.hook)
            payload["tool_input"] = f"{payload['tool_input']}\n"

            recorded = self.hook.process_post_tool_use(payload, Path(raw_dir))

            self.assertTrue(recorded)

    def test_self_reported_code_cannot_record_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            payload = _post_payload(self.hook)
            payload["tool_input"] = "text(JSON.stringify({status: 'unavailable'}));"

            recorded = self.hook.process_post_tool_use(payload, Path(raw_dir))

            self.assertFalse(recorded)

    def test_modified_probe_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            plugin_root = Path(raw_dir) / "plugin"
            probe = (
                plugin_root
                / "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs"
            )
            probe.parent.mkdir(parents=True)
            _ = probe.write_text("export const changed = true;\n", encoding="utf-8")
            payload = _post_payload(self.hook)
            payload["tool_input"] = self.hook.canonical_probe_code(plugin_root)

            recorded = self.hook.process_post_tool_use(
                payload, Path(raw_dir) / "data", plugin_root
            )

            self.assertFalse(recorded)

    def test_windows_line_endings_preserve_probe_integrity(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            plugin_root = Path(raw_dir) / "plugin"
            relative_probe = Path(
                "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs"
            )
            source_probe = HOOK_PATH.parent.parent / relative_probe
            probe = plugin_root / relative_probe
            probe.parent.mkdir(parents=True)
            source = source_probe.read_text(encoding="utf-8")
            _ = probe.write_bytes(source.replace("\n", "\r\n").encode("utf-8"))
            payload = _post_payload(self.hook)
            payload["tool_input"] = self.hook.canonical_probe_code(plugin_root)

            recorded = self.hook.process_post_tool_use(
                payload, Path(raw_dir) / "data", plugin_root
            )

            self.assertTrue(recorded)

    def test_unverified_or_available_evidence_does_not_authorize_claim(self) -> None:
        for status in ("unverified", "available"):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as raw_dir:
                data_dir = Path(raw_dir)
                self.assertTrue(
                    self.hook.process_post_tool_use(
                        _post_payload(self.hook, status), data_dir
                    )
                )

                output = self.hook.process_stop(
                    _stop_payload("Chrome unavailable."), data_dir
                )

                self.assertIsNotNone(output)
                if output is not None:
                    self.assertEqual(output["decision"], "block")

    def test_missing_or_wrong_turn_receipt_blocks_english_and_korean_claims(self) -> None:
        claims = (
            "Chrome is unavailable.",
            "크롬을 사용할 수 없습니다.",
        )
        for claim in claims:
            with self.subTest(claim=claim), tempfile.TemporaryDirectory() as raw_dir:
                output = self.hook.process_stop(
                    _stop_payload(claim, turn_id="different-turn"), Path(raw_dir)
                )

                self.assertIsNotNone(output)
                if output is not None:
                    self.assertEqual(output["decision"], "block")

    def test_unverified_phrase_and_unrelated_failures_are_allowed(self) -> None:
        messages = (
            "Chrome bootstrap was not verified.",
            "The ChatGPT composer is unavailable, but Chrome is connected.",
        )
        for message in messages:
            with self.subTest(message=message), tempfile.TemporaryDirectory() as raw_dir:
                self.assertIsNone(
                    self.hook.process_stop(_stop_payload(message), Path(raw_dir))
                )

    def test_unverified_phrase_cannot_mask_an_unavailable_claim(self) -> None:
        message = (
            "Chrome bootstrap was not verified, but Chrome is unavailable."
        )
        with tempfile.TemporaryDirectory() as raw_dir:
            output = self.hook.process_stop(_stop_payload(message), Path(raw_dir))

            self.assertIsNotNone(output)
            if output is not None:
                self.assertEqual(output["decision"], "block")

    def test_direct_node_tool_requires_exact_inner_probe(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            payload = _post_payload(self.hook)
            payload["tool_name"] = "mcp__node_repl__js"
            payload["tool_input"] = {"code": self.hook.canonical_inner_probe_code()}

            recorded = self.hook.process_post_tool_use(payload, Path(raw_dir))

            self.assertTrue(recorded)


if __name__ == "__main__":
    _ = unittest.main()
