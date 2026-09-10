from __future__ import annotations

import argparse
import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo
from codex_thread_settings import UnsupportedThreadSettingError


class DesktopBridgeSettingsCommandTests(unittest.TestCase):
    def test_command_settings_updates_thread_model_reasoning_and_speed(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_sidecar = bridge.CodexAppServerSidecar
        original_state_path = bridge.BRIDGE_STATE_PATH
        calls: list[tuple[str, str | None]] = []
        settings_calls: list[dict[str, str | None]] = []

        thread = ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="session.jsonl",
            model="gpt-5.5",
            reasoning_effort="xhigh",
            tokens_used=0,
        )

        class FakeSidecar:
            def list_models(self) -> dict[str, object]:
                calls.append(("models", None))
                return {
                    "data": [
                        {
                            "model": "gpt-5.4",
                            "hidden": False,
                            "supportedReasoningEfforts": [{"reasoningEffort": "high"}],
                            "additionalSpeedTiers": ["fast"],
                            "serviceTiers": [{"id": "priority", "name": "Fast"}],
                        }
                    ]
                }

            def resume_thread(self, thread_id: str) -> dict[str, str]:
                calls.append(("resume", thread_id))
                return {}

            def update_thread_settings(
                self,
                thread_id: str,
                settings: dict[str, str | None],
            ) -> dict[str, str]:
                calls.append(("settings", thread_id))
                settings_calls.append(dict(settings))
                return {}

            def close(self) -> None:
                calls.append(("close", None))

        try:
            with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
                bridge.BRIDGE_STATE_PATH = Path(temp_dir) / "state.json"
                bridge.choose_thread = lambda thread_id=None, cwd=None: thread
                bridge.CodexAppServerSidecar = FakeSidecar
                args = argparse.Namespace(
                    thread_ref=None,
                    thread_id="thread-1",
                    cwd=None,
                    model="gpt-5.4",
                    reasoning="high",
                    speed="fast",
                )

                output = io.StringIO()
                with redirect_stdout(output):
                    self.assertEqual(bridge.command_settings(args), 0)

                self.assertEqual(
                    calls,
                    [
                        ("models", None),
                        ("resume", "thread-1"),
                        ("settings", "thread-1"),
                        ("close", None),
                    ],
                )
                self.assertEqual(
                    settings_calls,
                    [{"model": "gpt-5.4", "effort": "high", "serviceTier": "priority"}],
                )
                text = output.getvalue()
                self.assertIn("model: gpt-5.4", text)
                self.assertIn("reasoning: high", text)
                self.assertIn("speed: fast", text)
                self.assertIn("transport: local-sidecar thread/settings/update", text)
                self.assertEqual(
                    bridge.get_saved_thread_settings("thread-1"),
                    {"model": "gpt-5.4", "reasoning": "high", "speed": "fast"},
                )
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.CodexAppServerSidecar = original_sidecar
            bridge.BRIDGE_STATE_PATH = original_state_path

    def test_settings_parser_accepts_app_provided_values_for_runtime_validation(self) -> None:
        parser = bridge.build_parser()

        _ = parser.parse_args(["settings", "--model", "gpt-5.4-mini", "--reasoning", "medium"])

    def test_command_settings_rejects_model_missing_from_app_catalog(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_sidecar = bridge.CodexAppServerSidecar

        thread = ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="session.jsonl",
            model="gpt-5.5",
            reasoning_effort="xhigh",
            tokens_used=0,
        )

        class FakeSidecar:
            def list_models(self) -> dict[str, object]:
                return {"data": [{"model": "gpt-5.5", "hidden": False}]}

            def resume_thread(self, _thread_id: str) -> dict[str, str]:
                return {}

            def update_thread_settings(
                self,
                _thread_id: str,
                _settings: dict[str, str | None],
            ) -> dict[str, str]:
                return {}

            def close(self) -> None:
                pass

        try:
            bridge.choose_thread = lambda thread_id=None, cwd=None: thread
            bridge.CodexAppServerSidecar = FakeSidecar
            args = argparse.Namespace(
                thread_ref=None,
                thread_id="thread-1",
                cwd=None,
                model="gpt-unknown",
                reasoning=None,
                speed=None,
            )

            with self.assertRaises(UnsupportedThreadSettingError):
                _ = bridge.command_settings(args)
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.CodexAppServerSidecar = original_sidecar

    def test_command_settings_options_reads_app_model_list(self) -> None:
        original_sidecar = bridge.CodexAppServerSidecar

        class FakeSidecar:
            def list_models(self) -> dict[str, object]:
                return {
                    "data": [
                        {
                            "model": "gpt-live",
                            "hidden": False,
                            "supportedReasoningEfforts": [{"reasoningEffort": "xhigh"}],
                            "additionalSpeedTiers": ["fast"],
                            "serviceTiers": [{"id": "priority", "name": "Fast"}],
                        }
                    ]
                }

            def close(self) -> None:
                pass

        try:
            bridge.CodexAppServerSidecar = FakeSidecar
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(bridge.command_settings_options(argparse.Namespace(field="model")), 0)

            text = output.getvalue()
            self.assertIn("models: gpt-live", text)
            self.assertIn("source: app model/list", text)
        finally:
            bridge.CodexAppServerSidecar = original_sidecar
