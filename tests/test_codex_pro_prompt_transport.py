from __future__ import annotations

import unittest
from threading import Event, Lock, Thread
from typing import override

import codex_discord_prompt_transport as prompt_transport
import codex_pro_browser_evidence as browser_evidence
import codex_pro_session_mirror_gate as mirror_gate
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


class ProPromptTransportTests(unittest.TestCase):
    @override
    def tearDown(self) -> None:
        mirror_gate.reset_for_tests()

    def test_chrome_failure_rejects_mirrored_turn_and_preserves_public_error(self) -> None:
        def complete(_target: str | None, _turn: str | None) -> None:
            raise browser_evidence.ProChromeUnavailableError("pro_chrome_unavailable")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            PRO_PROMPT,
            "thread-1",
            build_deps(
                enabled=False,
                start_turn_no_wait=lambda _prompt, target: app_server_delivery(
                    target,
                    "turn-1",
                ),
                complete_pro_browser_session=complete,
            ),
        )

        self.assertEqual(exit_code, 1)
        self.assertEqual(output, "pro_chrome_unavailable")
        self.assertEqual(mirror_gate.mode("thread-1"), mirror_gate.GateMode.DISCARD)

    def test_verified_chrome_releases_buffered_mirror_output(self) -> None:
        observed_modes: list[mirror_gate.GateMode] = []

        def complete(target: str | None, _turn: str | None) -> None:
            observed_modes.append(mirror_gate.mode(target))

        exit_code, _ = prompt_transport.run_transport_prompt_no_wait(
            PRO_PROMPT,
            "thread-1",
            build_deps(
                enabled=False,
                start_turn_no_wait=lambda _prompt, target: app_server_delivery(
                    target,
                    "turn-1",
                ),
                complete_pro_browser_session=complete,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(observed_modes, [mirror_gate.GateMode.HOLD])
        self.assertEqual(mirror_gate.mode("thread-1"), mirror_gate.GateMode.OPEN)

    def test_concurrent_pro_prompts_serialize_app_server_delivery(self) -> None:
        events: list[str] = []
        events_lock = Lock()
        first_activated = Event()
        second_prepare_entered = Event()

        def record(event: str) -> None:
            with events_lock:
                events.append(event)

        def prepare(target_thread_id: str | None) -> None:
            record(f"activate:{target_thread_id}")
            if target_thread_id == "thread-1":
                first_activated.set()
            else:
                second_prepare_entered.set()

        def start_turn(_prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            if target_thread_id == "thread-1":
                _ = second_prepare_entered.wait(timeout=0.5)
            record(f"app-server:{target_thread_id}")
            return app_server_delivery(
                target_thread_id,
                f"turn-{target_thread_id}",
            )

        def complete(target_thread_id: str | None, turn_id: str | None) -> None:
            record(f"complete:{target_thread_id}:{turn_id}")

        deps = build_deps(
            enabled=False,
            prepare_pro_browser_session=prepare,
            complete_pro_browser_session=complete,
            start_turn_no_wait=start_turn,
        )

        first = Thread(
            target=prompt_transport.run_transport_prompt_no_wait,
            args=(PRO_PROMPT, "thread-1", deps),
        )

        def run_second() -> None:
            self.assertTrue(first_activated.wait(timeout=1.0))
            _ = prompt_transport.run_transport_prompt_no_wait(PRO_PROMPT, "thread-2", deps)

        second = Thread(target=run_second)
        first.start()
        second.start()
        first.join(timeout=2.0)
        second.join(timeout=2.0)

        self.assertFalse(first.is_alive())
        self.assertFalse(second.is_alive())
        self.assertEqual(
            events,
            [
                "activate:thread-1",
                "app-server:thread-1",
                "complete:thread-1:turn-thread-1",
                "activate:thread-2",
                "app-server:thread-2",
                "complete:thread-2:turn-thread-2",
            ],
        )

    def test_pro_paths_require_same_turn_browser_completion(self) -> None:
        completions: list[tuple[str | None, str | None]] = []

        exit_code, _ = prompt_transport.run_transport_prompt_no_wait(
            PRO_PROMPT,
            "thread-1",
            build_deps(
                enabled=False,
                start_turn_no_wait=lambda _prompt, target: app_server_delivery(
                    target,
                    "turn-no-wait",
                ),
                complete_pro_browser_session=lambda target, turn: completions.append(
                    (target, turn)
                ),
            ),
        )

        relay = FakeRelay()
        stream_exit_code, _ = prompt_transport.run_ask_stream(
            PRO_PROMPT,
            relay,
            wait=False,
            target_thread_id="thread-2",
            deps=build_deps(
                enabled=False,
                start_turn_no_wait=lambda _prompt, target: app_server_delivery(
                    target,
                    "turn-stream",
                ),
                complete_pro_browser_session=lambda target, turn: completions.append(
                    (target, turn)
                ),
                run_watch_stream=finish_watch,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(stream_exit_code, 0)
        self.assertEqual(
            completions,
            [("thread-1", "turn-no-wait"), ("thread-2", "turn-stream")],
        )

if __name__ == "__main__":
    _ = unittest.main()
