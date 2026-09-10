from __future__ import annotations

import hashlib
import json
import shutil
import tempfile
import unittest
from pathlib import Path

import codex_pro_browser_evidence as browser_evidence


class ProBrowserEvidenceTests(unittest.TestCase):
    def test_exact_turn_transcript_accepts_trusted_available_probe(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            root = Path(raw_dir)
            plugin_root = root / "plugin"
            probe = plugin_root / browser_evidence.PROBE_RELATIVE_PATH
            probe.parent.mkdir(parents=True)
            shutil.copyfile(
                Path("plugins/codex-discord-remote")
                / browser_evidence.PROBE_RELATIVE_PATH,
                probe,
            )
            session_path = (
                root
                / "sessions/2026/08/13"
                / "rollout-test-session-a.jsonl"
            )
            session_path.parent.mkdir(parents=True)
            tool_input = browser_evidence.canonical_probe_code(plugin_root)
            evidence = {
                "protocol": browser_evidence.PROTOCOL,
                "browser_type": "chrome",
                "status": "available",
                "can_report_unavailable": False,
            }
            records = [
                {
                    "type": "response_item",
                    "payload": {
                        "type": "custom_tool_call",
                        "call_id": "call-a",
                        "name": "exec",
                        "input": tool_input,
                        "internal_chat_message_metadata_passthrough": {
                            "turn_id": "turn-a"
                        },
                    },
                },
                {
                    "type": "response_item",
                    "payload": {
                        "type": "custom_tool_call_output",
                        "call_id": "call-a",
                        "output": [{"type": "input_text", "text": json.dumps(evidence)}],
                        "internal_chat_message_metadata_passthrough": {
                            "turn_id": "turn-a"
                        },
                    },
                },
            ]
            _ = session_path.write_text(
                "".join(json.dumps(record) + "\n" for record in records),
                encoding="utf-8",
            )

            browser_evidence.require_available_evidence(
                "session-a",
                "turn-a",
                plugin_data=root / "missing-plugin-data",
                codex_home=root,
                plugin_roots=(plugin_root,),
            )
            with self.assertRaisesRegex(
                browser_evidence.ProChromeUnavailableError,
                "^pro_chrome_unavailable$",
            ):
                browser_evidence.require_available_evidence(
                    "session-a",
                    "turn-b",
                    plugin_data=root / "missing-plugin-data",
                    codex_home=root,
                    plugin_roots=(plugin_root,),
                )

    def test_available_receipt_is_scoped_to_exact_session_and_turn(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            data_dir = Path(raw_dir)
            receipt_dir = data_dir / "browser-evidence"
            receipt_dir.mkdir()
            key = hashlib.sha256(b"session-a\0turn-a").hexdigest()
            _ = (receipt_dir / f"{key}.json").write_text(
                json.dumps(
                    {
                        "protocol": browser_evidence.PROTOCOL,
                        "session_id": "session-a",
                        "turn_id": "turn-a",
                        "browser_type": "chrome",
                        "status": "available",
                        "can_report_unavailable": False,
                        "probe_sha256": browser_evidence.EXPECTED_PROBE_SHA256,
                    }
                ),
                encoding="utf-8",
            )

            browser_evidence.require_available_evidence(
                "session-a",
                "turn-a",
                plugin_data=data_dir,
            )
            with self.assertRaisesRegex(
                browser_evidence.ProChromeUnavailableError,
                "^pro_chrome_unavailable$",
            ):
                browser_evidence.require_available_evidence(
                    "session-a",
                    "turn-b",
                    plugin_data=data_dir,
                )

    def test_unavailable_or_unverified_receipt_fails_closed(self) -> None:
        for status in ("unavailable", "unverified"):
            with self.subTest(status=status), tempfile.TemporaryDirectory() as raw_dir:
                data_dir = Path(raw_dir)
                receipt_dir = data_dir / "browser-evidence"
                receipt_dir.mkdir()
                key = hashlib.sha256(b"session-a\0turn-a").hexdigest()
                _ = (receipt_dir / f"{key}.json").write_text(
                    json.dumps(
                        {
                            "protocol": browser_evidence.PROTOCOL,
                            "session_id": "session-a",
                            "turn_id": "turn-a",
                            "browser_type": "chrome",
                            "status": status,
                            "can_report_unavailable": status == "unavailable",
                            "probe_sha256": browser_evidence.EXPECTED_PROBE_SHA256,
                        }
                    ),
                    encoding="utf-8",
                )

                with self.assertRaisesRegex(
                    browser_evidence.ProChromeUnavailableError,
                    "^pro_chrome_unavailable$",
                ):
                    browser_evidence.require_available_evidence(
                        "session-a",
                        "turn-a",
                        plugin_data=data_dir,
                    )


if __name__ == "__main__":
    _ = unittest.main()
