from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias
import unittest

import codex_discord_prompt_transport as prompt_transport


PromptNoWaitFunc: TypeAlias = Callable[[str, str | None], tuple[int, str]]
PrepareProBrowserSessionFunc: TypeAlias = Callable[[str | None], None]
CompleteProBrowserSessionFunc: TypeAlias = Callable[[str | None, str | None], None]
StartTurnNoWaitFunc: TypeAlias = Callable[[str, str | None], "FakeDeliveryResult"]
LogFunc: TypeAlias = Callable[[str], None]
WatchStreamFunc: TypeAlias = Callable[["FakeSteeringResult", "FakeRelay"], tuple[int, str]]


class LegacyStreamFunc(Protocol):
    def __call__(
        self,
        prompt: str,
        relay: FakeRelay,
        *,
        force_while_busy: bool = False,
        wait: bool = True,
        target_thread_id: str | None = None,
    ) -> tuple[int, str]: ...


@dataclass(frozen=True, slots=True)
class FakeDeliveryResult:
    exit_code: int
    output: str
    thread_id: str | None = None
    turn_id: str | None = None
    target_ref: str = ""
    session_path: str | None = None
    start_offset: int | None = None
    delivery_pending: bool = False


@dataclass(frozen=True, slots=True)
class FakeSteeringResult:
    exit_code: int
    output: str
    target_thread_id: str | None = None
    target_ref: str = ""
    session_path: str | None = None
    start_offset: int | None = None
    delivery_pending: bool = False


class FakeRelay:
    def __init__(self) -> None:
        self.finished: bool = False
        self.lines: list[str] = []

    def feed_line(self, line: str) -> None:
        self.lines.append(line)

    def finish(self) -> None:
        self.finished = True


def make_steering_result(delivery: FakeDeliveryResult) -> FakeSteeringResult:
    return FakeSteeringResult(
        delivery.exit_code,
        delivery.output,
        target_thread_id=delivery.thread_id,
        target_ref=delivery.target_ref,
        session_path=delivery.session_path,
        start_offset=delivery.start_offset,
        delivery_pending=delivery.delivery_pending,
    )


def build_deps(
    *,
    enabled: bool = True,
    run_resident_prompt_no_wait: PromptNoWaitFunc | None = None,
    run_legacy_prompt_no_wait: PromptNoWaitFunc | None = None,
    prepare_pro_browser_session: PrepareProBrowserSessionFunc | None = None,
    complete_pro_browser_session: CompleteProBrowserSessionFunc | None = None,
    start_turn_no_wait: StartTurnNoWaitFunc | None = None,
    run_watch_stream: WatchStreamFunc | None = None,
    run_legacy_stream: LegacyStreamFunc | None = None,
    log: LogFunc | None = None,
) -> prompt_transport.PromptTransportDeps[FakeRelay, FakeDeliveryResult, FakeSteeringResult]:
    def app_server_transport_enabled() -> bool:
        return enabled

    def unexpected_prompt_no_wait(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        raise AssertionError(f"unexpected prompt transport: {prompt} {target_thread_id}")

    def unexpected_start_turn(prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
        raise AssertionError(f"unexpected start turn: {prompt} {target_thread_id}")

    def unexpected_watch(steering_result: FakeSteeringResult, relay: FakeRelay) -> tuple[int, str]:
        raise AssertionError(f"unexpected watch: {steering_result} {relay}")

    def unexpected_legacy_stream(
        prompt: str,
        relay: FakeRelay,
        *,
        force_while_busy: bool = False,
        wait: bool = True,
        target_thread_id: str | None = None,
    ) -> tuple[int, str]:
        raise AssertionError(
            f"unexpected legacy stream: {prompt} {relay} {force_while_busy} {wait} {target_thread_id}"
        )

    def discard_log(message: str) -> None:
        _ = message

    def discard_pro_browser_session(_target_thread_id: str | None) -> None:
        return

    def discard_pro_browser_completion(
        _target_thread_id: str | None,
        _turn_id: str | None,
    ) -> None:
        return

    return prompt_transport.PromptTransportDeps(
        app_server_transport_enabled=app_server_transport_enabled,
        prepare_pro_browser_session=prepare_pro_browser_session or discard_pro_browser_session,
        complete_pro_browser_session=complete_pro_browser_session or discard_pro_browser_completion,
        run_resident_prompt_no_wait=run_resident_prompt_no_wait or unexpected_prompt_no_wait,
        run_legacy_prompt_no_wait=run_legacy_prompt_no_wait or unexpected_prompt_no_wait,
        start_turn_no_wait=start_turn_no_wait or unexpected_start_turn,
        make_steering_prompt_result=make_steering_result,
        run_watch_stream=run_watch_stream or unexpected_watch,
        run_legacy_stream=run_legacy_stream or unexpected_legacy_stream,
        log=log or discard_log,
    )


class PromptTransportTests(unittest.TestCase):
    def test_run_transport_prompt_no_wait_uses_legacy_when_disabled(self) -> None:
        calls: list[tuple[str, str | None]] = []

        def legacy(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            calls.append((prompt, target_thread_id))
            return 0, "legacy ipc"

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(enabled=False, run_legacy_prompt_no_wait=legacy),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "legacy ipc")
        self.assertEqual(calls, [("please run", "thread-1")])

    def test_run_transport_prompt_no_wait_does_not_activate_browser_for_normal_prompt(self) -> None:
        activations: list[str | None] = []

        def resident(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            return 0, f"resident: {prompt} {target_thread_id}"

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(
                enabled=True,
                prepare_pro_browser_session=activations.append,
                run_resident_prompt_no_wait=resident,
            ),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "resident: please run thread-1")
        self.assertEqual(activations, [])

    def test_run_transport_prompt_no_wait_logs_resident_exception_without_fallback(self) -> None:
        logs: list[str] = []

        def resident(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise RuntimeError(f"transport boom: {prompt} {target_thread_id}")

        def legacy(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise AssertionError(f"must not fall back to legacy: {prompt} {target_thread_id}")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(
                run_resident_prompt_no_wait=resident,
                run_legacy_prompt_no_wait=legacy,
                log=logs.append,
            ),
        )

        self.assertEqual(exit_code, 1)
        self.assertEqual(
            output,
            "ERROR: resident app-server transport failed: transport boom: please run thread-1",
        )
        self.assertEqual(len(logs), 1)
        self.assertIn("app_server_prompt_failed target=thread-1", logs[0])
        self.assertIn("error_type=RuntimeError", logs[0])

    def test_run_transport_prompt_no_wait_surfaces_rollout_thread_id_parse_error(self) -> None:
        logs: list[str] = []

        def resident(_prompt: str, _target_thread_id: str | None) -> tuple[int, str]:
            raise RuntimeError(
                "thread/resume failed: failed to load rollout "
                + "C:\\Users\\example\\.codex\\sessions\\2026\\07\\04\\rollout.jsonl: "
                + "failed to parse thread ID from rollout file"
            )

        def legacy(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            raise AssertionError(f"must not fall back to IPC: {prompt} {target_thread_id}")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(
                run_resident_prompt_no_wait=resident,
                run_legacy_prompt_no_wait=legacy,
                log=logs.append,
            ),
        )

        self.assertEqual(exit_code, 1)
        self.assertIn("ERROR: resident app-server transport failed:", output)
        self.assertIn("failed to parse thread ID from rollout file", output)
        self.assertEqual(len(logs), 1)
        self.assertIn("app_server_prompt_failed target=thread-1", logs[0])

    def test_run_transport_prompt_no_wait_explains_thread_not_found_split_brain(self) -> None:
        def resident(_prompt: str, _target_thread_id: str | None) -> tuple[int, str]:
            raise RuntimeError("turn/start failed: Thread not found: thread-1")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(run_resident_prompt_no_wait=resident),
        )

        self.assertEqual(exit_code, 1)
        self.assertIn("ERROR: resident app-server transport failed:", output)
        self.assertIn("resident app-server cannot open it", output)
        self.assertIn("Run `!mirror sync`", output)

    def test_thread_resume_timeout_suggests_manual_resume_without_replaying_prompt(self) -> None:
        def resident(_prompt: str, _target_thread_id: str | None) -> tuple[int, str]:
            raise TimeoutError("Timed out waiting for app-server response to thread/resume.")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(run_resident_prompt_no_wait=resident),
        )

        self.assertEqual(exit_code, 1)
        self.assertIn("`!resume`", output)

    def test_thread_read_timeout_explains_request_local_failure_and_recovery(self) -> None:
        def resident(_prompt: str, _target_thread_id: str | None) -> tuple[int, str]:
            raise TimeoutError("Timed out waiting for app-server response to thread/read.")

        exit_code, output = prompt_transport.run_transport_prompt_no_wait(
            "please run",
            "thread-1",
            build_deps(run_resident_prompt_no_wait=resident),
        )

        self.assertEqual(exit_code, 1)
        self.assertIn("did not stop or restart other Codex threads", output)
        self.assertIn("Run `!resume`", output)
        self.assertIn("The failed prompt was not sent", output)

    def test_run_ask_stream_watches_app_server_session_when_waiting(self) -> None:
        relay = FakeRelay()
        watched: list[FakeSteeringResult] = []

        def start_turn(prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            self.assertEqual(prompt, "please run")
            self.assertEqual(target_thread_id, "thread-1")
            return FakeDeliveryResult(
                0,
                "started",
                thread_id="thread-1",
                target_ref="taxlab:1",
                session_path="session.jsonl",
                start_offset=10,
                delivery_pending=True,
            )

        def watch(steering_result: FakeSteeringResult, relay_arg: FakeRelay) -> tuple[int, str]:
            watched.append(steering_result)
            relay_arg.finish()
            return 0, "watched"

        exit_code, output = prompt_transport.run_ask_stream(
            "please run",
            relay,
            target_thread_id="thread-1",
            deps=build_deps(start_turn_no_wait=start_turn, run_watch_stream=watch),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "watched")
        self.assertTrue(relay.finished)
        self.assertEqual(
            watched,
            [
                FakeSteeringResult(
                    0,
                    "started",
                    target_thread_id="thread-1",
                    target_ref="taxlab:1",
                    session_path="session.jsonl",
                    start_offset=10,
                    delivery_pending=True,
                )
            ],
        )

    def test_run_ask_stream_finishes_and_returns_result_without_watch_when_no_wait(self) -> None:
        relay = FakeRelay()

        def start_turn(prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            return FakeDeliveryResult(0, f"started: {prompt} {target_thread_id}")

        exit_code, output = prompt_transport.run_ask_stream(
            "please run",
            relay,
            wait=False,
            target_thread_id="thread-1",
            deps=build_deps(start_turn_no_wait=start_turn),
        )

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "started: please run thread-1")
        self.assertTrue(relay.finished)

    def test_run_ask_stream_uses_legacy_stream_when_disabled(self) -> None:
        relay = FakeRelay()
        calls: list[tuple[str, bool, bool, str | None]] = []

        def legacy_stream(
            prompt: str,
            relay: FakeRelay,
            *,
            force_while_busy: bool = False,
            wait: bool = True,
            target_thread_id: str | None = None,
        ) -> tuple[int, str]:
            calls.append((prompt, force_while_busy, wait, target_thread_id))
            relay.finish()
            return 2, "legacy stream"

        exit_code, output = prompt_transport.run_ask_stream(
            "please run",
            relay,
            force_while_busy=True,
            wait=False,
            target_thread_id="thread-1",
            deps=build_deps(enabled=False, run_legacy_stream=legacy_stream),
        )

        self.assertEqual(exit_code, 2)
        self.assertEqual(output, "legacy stream")
        self.assertTrue(relay.finished)
        self.assertEqual(calls, [("please run", True, False, "thread-1")])

    def test_run_ask_stream_logs_finishes_and_skips_fallback_on_start_exception(self) -> None:
        relay = FakeRelay()
        logs: list[str] = []

        def start_turn(prompt: str, target_thread_id: str | None) -> FakeDeliveryResult:
            raise RuntimeError(f"stream boom: {prompt} {target_thread_id}")

        def legacy_stream(
            prompt: str,
            relay: FakeRelay,
            *,
            force_while_busy: bool = False,
            wait: bool = True,
            target_thread_id: str | None = None,
        ) -> tuple[int, str]:
            _ = force_while_busy
            raise AssertionError(f"must not fall back to stream: {prompt} {relay} {wait} {target_thread_id}")

        exit_code, output = prompt_transport.run_ask_stream(
            "please run",
            relay,
            target_thread_id="thread-1",
            deps=build_deps(
                start_turn_no_wait=start_turn,
                run_legacy_stream=legacy_stream,
                log=logs.append,
            ),
        )

        self.assertEqual(exit_code, 1)
        self.assertEqual(
            output,
            "ERROR: resident app-server transport failed: stream boom: please run thread-1",
        )
        self.assertTrue(relay.finished)
        self.assertEqual(len(logs), 1)
        self.assertIn("app_server_stream_prompt_failed target=thread-1", logs[0])
        self.assertIn("error_type=RuntimeError", logs[0])
