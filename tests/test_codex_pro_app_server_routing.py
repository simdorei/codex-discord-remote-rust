from __future__ import annotations

import unittest

import codex_discord_prompt_transport as prompt_transport
from tests.test_codex_discord_prompt_transport import (
    FakeDeliveryResult,
    FakeRelay,
    FakeSteeringResult,
    build_deps,
)


PRO_PROMPT = "$ask-chatgpt-pro [@Chrome](plugin://chrome@openai-bundled)"


def app_server_delivery(target_thread_id: str | None, turn_id: str) -> FakeDeliveryResult:
    return FakeDeliveryResult(
        0,
        f"[app_server_delivery] turn_id={turn_id}",
        thread_id=target_thread_id,
        turn_id=turn_id,
        session_path="session.jsonl",
        start_offset=10,
    )


def finish_watch(_result: FakeSteeringResult, relay: FakeRelay) -> tuple[int, str]:
    relay.finish()
    return 0, "watched"


class ProAppServerRoutingTests(unittest.TestCase):
    def test_pro_prompt_no_wait_uses_app_server_delivery_identity(self) -> None:
        events: list[str] = []

        def start_turn(
            prompt: str,
            target_thread_id: str | None,
        ) -> FakeDeliveryResult:
            events.append(f"app-server:{target_thread_id}:{prompt}")
            return app_server_delivery(target_thread_id, "turn-app-server")

        def complete(target_thread_id: str | None, turn_id: str | None) -> None:
            events.append(f"complete:{target_thread_id}:{turn_id}")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            PRO_PROMPT,
            "thread-1",
            build_deps(
                enabled=True,
                start_turn_no_wait=start_turn,
                complete_pro_browser_session=complete,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "[app_server_delivery] turn_id=turn-app-server")
        self.assertEqual(
            events,
            [
                f"app-server:thread-1:{PRO_PROMPT}",
                "complete:thread-1:turn-app-server",
            ],
        )

    def test_pro_prompt_no_wait_admits_target_before_app_server_turn(self) -> None:
        events: list[str] = []

        def prepare(target_thread_id: str | None) -> None:
            events.append(f"activate:{target_thread_id}")

        def start_turn(_prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            events.append(f"app-server:{target_thread_id}")
            return app_server_delivery(target_thread_id, "turn-1")

        exit_code, _ = prompt_transport.run_transport_prompt_no_wait(
            PRO_PROMPT,
            "thread-1",
            build_deps(
                enabled=False,
                prepare_pro_browser_session=prepare,
                start_turn_no_wait=start_turn,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(events, ["activate:thread-1", "app-server:thread-1"])

    def test_pro_stream_admits_target_before_app_server_turn(self) -> None:
        relay = FakeRelay()
        events: list[str] = []

        def prepare(target_thread_id: str | None) -> None:
            events.append(f"activate:{target_thread_id}")

        def start_turn(_prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            events.append(f"app-server:{target_thread_id}")
            return app_server_delivery(target_thread_id, "turn-1")

        exit_code, _ = prompt_transport.run_ask_stream(
            PRO_PROMPT,
            relay,
            wait=False,
            target_thread_id="thread-1",
            deps=build_deps(
                enabled=False,
                prepare_pro_browser_session=prepare,
                start_turn_no_wait=start_turn,
                run_watch_stream=finish_watch,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(events, ["activate:thread-1", "app-server:thread-1"])

    def test_pro_prompt_no_wait_uses_app_server_when_resident_is_enabled(self) -> None:
        calls: list[tuple[str, str | None]] = []

        def start_turn(prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            calls.append((prompt, target_thread_id))
            return app_server_delivery(target_thread_id, "turn-1")

        def resident(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise AssertionError(f"unexpected resident transport: {prompt}:{target_thread_id}")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            PRO_PROMPT,
            "thread-1",
            build_deps(
                enabled=True,
                run_resident_prompt_no_wait=resident,
                start_turn_no_wait=start_turn,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "[app_server_delivery] turn_id=turn-1")
        self.assertEqual(calls, [(PRO_PROMPT, "thread-1")])

    def test_pro_stream_uses_app_server_when_resident_is_enabled(self) -> None:
        relay = FakeRelay()
        calls: list[tuple[str, str | None]] = []

        def start_turn(prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            calls.append((prompt, target_thread_id))
            return app_server_delivery(target_thread_id, "turn-1")

        def resident(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise AssertionError(f"unexpected resident transport: {prompt}:{target_thread_id}")

        exit_code, output = prompt_transport.run_ask_stream(
            PRO_PROMPT,
            relay,
            force_while_busy=True,
            wait=False,
            target_thread_id="thread-1",
            deps=build_deps(
                enabled=True,
                run_resident_prompt_no_wait=resident,
                start_turn_no_wait=start_turn,
                run_watch_stream=finish_watch,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "watched")
        self.assertTrue(relay.finished)
        self.assertEqual(relay.lines, [])
        self.assertEqual(calls, [(PRO_PROMPT, "thread-1")])


if __name__ == "__main__":
    _ = unittest.main()
