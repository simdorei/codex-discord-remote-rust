from __future__ import annotations

from collections.abc import Callable
from threading import Lock
import unittest

import codex_app_server_transport as app_server_transport
import codex_desktop_bridge as bridge
import codex_discord_prompt_transport as prompt_transport
import codex_discord_prompt_transport_factory as prompt_transport_factory
from codex_pro_prompt_contract import PRO_SKILL_CALL
from tests.test_codex_discord_prompt_transport_factory import (
    FactoryRelay,
    FakeSteeringResult,
    make_app_steering_result,
)


class ProAppServerFactoryTests(unittest.TestCase):
    def test_pro_no_wait_uses_resident_app_server_without_ipc(self) -> None:
        bridge_stream_calls: list[list[str]] = []
        app_server_calls: list[tuple[str, str | None]] = []
        completion_calls: list[tuple[str | None, str | None]] = []

        def start_turn(
            prompt: str,
            target_thread_id: str | None,
        ) -> app_server_transport.AppServerDeliveryResult:
            app_server_calls.append((prompt, target_thread_id))
            return app_server_transport.AppServerDeliveryResult(
                0,
                "[app_server_delivery] turn_id=turn-1",
                thread_id=target_thread_id,
                turn_id="turn-1",
                session_path="session.jsonl",
                start_offset=4,
            )

        def legacy_prompt(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise AssertionError(f"unexpected legacy prompt: {prompt}:{target_thread_id}")

        def watch(
            steering_result: FakeSteeringResult,
            relay: FactoryRelay,
        ) -> tuple[int, str]:
            _ = steering_result
            relay.finish()
            return 0, "watched"

        def bridge_stream(argv: list[str], on_line: Callable[[str], None]) -> tuple[int, str]:
            _ = on_line
            bridge_stream_calls.append(argv)
            return 1, "unexpected legacy bridge call"

        deps = prompt_transport_factory.make_prompt_transport_deps(
            bridge_module=bridge,
            app_server_transport_enabled=lambda: True,
            run_legacy_prompt_no_wait=legacy_prompt,
            make_steering_prompt_result=make_app_steering_result,
            run_watch_stream=watch,
            run_bridge_command_stream=bridge_stream,
            ui_fallback_lock=Lock(),
            log=lambda message: None,
            start_turn_no_wait=start_turn,
            complete_pro_browser_session=lambda target, turn: completion_calls.append(
                (target, turn)
            ),
        )

        result = prompt_transport.run_transport_prompt_no_wait(
            PRO_SKILL_CALL,
            "thread-1",
            deps,
        )

        self.assertEqual(
            result,
            (0, "[app_server_delivery] turn_id=turn-1"),
        )
        self.assertEqual(app_server_calls, [(PRO_SKILL_CALL, "thread-1")])
        self.assertEqual(completion_calls, [("thread-1", "turn-1")])
        self.assertEqual(bridge_stream_calls, [])

    def test_pro_stream_uses_resident_app_server_without_ipc(self) -> None:
        bridge_stream_calls: list[list[str]] = []
        completion_calls: list[tuple[str | None, str | None]] = []

        def start_turn(
            _prompt: str,
            target_thread_id: str | None,
        ) -> app_server_transport.AppServerDeliveryResult:
            return app_server_transport.AppServerDeliveryResult(
                0,
                "[app_server_delivery] turn_id=turn-1",
                thread_id=target_thread_id,
                turn_id="turn-1",
                session_path="session.jsonl",
                start_offset=4,
            )

        def legacy_prompt(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise AssertionError(f"unexpected legacy prompt: {prompt}:{target_thread_id}")

        def watch(
            steering_result: FakeSteeringResult,
            relay: FactoryRelay,
        ) -> tuple[int, str]:
            _ = steering_result
            relay.finish()
            return 0, "watched"

        def bridge_stream(argv: list[str], on_line: Callable[[str], None]) -> tuple[int, str]:
            _ = on_line
            bridge_stream_calls.append(argv)
            return 1, "unexpected legacy bridge call"

        deps = prompt_transport_factory.make_prompt_transport_deps(
            bridge_module=bridge,
            app_server_transport_enabled=lambda: True,
            run_legacy_prompt_no_wait=legacy_prompt,
            make_steering_prompt_result=make_app_steering_result,
            run_watch_stream=watch,
            run_bridge_command_stream=bridge_stream,
            ui_fallback_lock=Lock(),
            log=lambda message: None,
            start_turn_no_wait=start_turn,
            complete_pro_browser_session=lambda target, turn: completion_calls.append(
                (target, turn)
            ),
        )
        relay = FactoryRelay()

        result = prompt_transport.run_ask_stream(
            PRO_SKILL_CALL,
            relay,
            target_thread_id="thread-1",
            deps=deps,
        )

        self.assertEqual(result, (0, "watched"))
        self.assertTrue(relay.finished)
        self.assertEqual(relay.lines, [])
        self.assertEqual(completion_calls, [("thread-1", "turn-1")])
        self.assertEqual(bridge_stream_calls, [])


if __name__ == "__main__":
    _ = unittest.main()
