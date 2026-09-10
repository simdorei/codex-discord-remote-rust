from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from threading import Lock
from typing import cast
import unittest
from unittest import mock

import codex_app_server_transport as app_server_transport
import codex_app_server_transport_delivery as app_server_delivery
import codex_desktop_bridge_protocol as bridge_protocol
import codex_discord_app_server as discord_app_server
import codex_discord_prompt_transport_factory as prompt_transport_factory
import codex_discord_stream as discord_stream


@dataclass(frozen=True, slots=True)
class FakeSteeringResult:
    exit_code: int
    output: str
    target_thread_id: str | None = None
    target_ref: str = ""
    session_path: str | None = None
    start_offset: int | None = None
    delivery_pending: bool = False


class FactoryRelay:
    def __init__(self) -> None:
        self.finished: bool = False
        self.lines: list[str] = []

    def feed_line(self, line: str) -> None:
        self.lines.append(line)

    def finish(self) -> None:
        self.finished = True


def make_app_steering_result(
    delivery: app_server_transport.AppServerDeliveryResult,
) -> FakeSteeringResult:
    return FakeSteeringResult(
        delivery.exit_code,
        delivery.output,
        target_thread_id=delivery.thread_id,
        target_ref=delivery.target_ref,
        session_path=delivery.session_path,
        start_offset=delivery.start_offset,
        delivery_pending=delivery.delivery_pending,
    )


class PromptTransportFactoryTests(unittest.TestCase):
    def test_make_prompt_transport_deps_wires_runtime_wrappers(self) -> None:
        legacy_prompt_calls: list[tuple[str, str | None]] = []
        bridge_stream_calls: list[list[str]] = []
        watched: list[FakeSteeringResult] = []
        bridge_module = cast(app_server_delivery.BridgeModule, cast(object, type("FakeBridge", (), {})()))

        def resident_prompt(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            return 0, f"resident:{prompt}:{target_thread_id}"

        def legacy_prompt(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            legacy_prompt_calls.append((prompt, target_thread_id))
            return 0, "legacy"

        def start_turn(prompt: str, target_thread_id: str | None) -> app_server_transport.AppServerDeliveryResult:
            return app_server_transport.AppServerDeliveryResult(
                0,
                f"started:{prompt}:{target_thread_id}",
                thread_id=target_thread_id,
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=4,
            )

        def watch(steering_result: FakeSteeringResult, relay: discord_stream.DiscordAskRelay) -> tuple[int, str]:
            watched.append(steering_result)
            relay.finish()
            return 0, "watched"

        def bridge_stream(argv: list[str], on_line: Callable[[str], None]) -> tuple[int, str]:
            bridge_stream_calls.append(argv)
            on_line("line")
            return 3, "legacy stream"

        deps = prompt_transport_factory.make_prompt_transport_deps(
            bridge_module=bridge_module,
            app_server_transport_enabled=lambda: True,
            run_legacy_prompt_no_wait=legacy_prompt,
            make_steering_prompt_result=make_app_steering_result,
            run_watch_stream=watch,
            run_bridge_command_stream=bridge_stream,
            ui_fallback_lock=Lock(),
            log=lambda message: None,
            run_resident_prompt_no_wait=resident_prompt,
            start_turn_no_wait=start_turn,
        )

        with mock.patch.object(
            bridge_protocol,
            "open_codex_thread_deep_link",
        ) as open_deep_link:
            deps.prepare_pro_browser_session("thread-browser")
        open_deep_link.assert_not_called()
        with self.assertRaisesRegex(RuntimeError, "requires a mapped Codex thread"):
            deps.prepare_pro_browser_session(None)

        self.assertEqual(deps.run_resident_prompt_no_wait("p", "thread-1"), (0, "resident:p:thread-1"))
        self.assertEqual(deps.run_legacy_prompt_no_wait("p", "thread-2"), (0, "legacy"))
        self.assertEqual(legacy_prompt_calls, [("p", "thread-2")])

        relay = FactoryRelay()
        stream_relay = cast(discord_stream.DiscordAskRelay, cast(object, relay))
        self.assertEqual(deps.run_legacy_stream("p", stream_relay, target_thread_id="thread-3"), (3, "legacy stream"))
        self.assertTrue(relay.finished)
        self.assertEqual(relay.lines, ["line"])
        self.assertEqual(bridge_stream_calls[0][:4], ["ask", "--ipc", "--foreground", "--stream"])

        relay = FactoryRelay()
        stream_relay = cast(discord_stream.DiscordAskRelay, cast(object, relay))
        self.assertEqual(
            deps.run_watch_stream(make_app_steering_result(start_turn("p", "thread-4")), stream_relay),
            (0, "watched"),
        )
        self.assertTrue(relay.finished)
        self.assertEqual(watched[0].target_thread_id, "thread-4")

    def test_default_stream_delivery_uses_steer_or_start(self) -> None:
        bridge_module = cast(app_server_delivery.BridgeModule, cast(object, type("FakeBridge", (), {})()))
        expected = app_server_transport.AppServerDeliveryResult(
            0,
            "delivered",
            thread_id="thread-1",
            target_ref="project:1",
            session_path="session.jsonl",
            start_offset=4,
        )

        def legacy_prompt(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            return 0, f"legacy:{prompt}:{target_thread_id}"

        def watch(steering_result: FakeSteeringResult, relay: discord_stream.DiscordAskRelay) -> tuple[int, str]:
            _ = relay
            return 0, steering_result.output

        def bridge_stream(argv: list[str], on_line: Callable[[str], None]) -> tuple[int, str]:
            _ = argv
            _ = on_line
            return 0, "legacy stream"

        def discard_log(message: str) -> None:
            _ = message

        with (
            mock.patch.dict("os.environ", {"DISCORD_STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS": ""}),
            mock.patch.object(
                app_server_transport,
                "steer_or_start_no_wait",
                return_value=expected,
            ) as steer_or_start,
            mock.patch.object(app_server_transport, "start_turn_no_wait") as start_turn,
        ):
            deps = prompt_transport_factory.make_prompt_transport_deps(
                bridge_module=bridge_module,
                app_server_transport_enabled=lambda: True,
                run_legacy_prompt_no_wait=legacy_prompt,
                make_steering_prompt_result=make_app_steering_result,
                run_watch_stream=watch,
                run_bridge_command_stream=bridge_stream,
                ui_fallback_lock=Lock(),
                log=discard_log,
            )

            result = deps.start_turn_no_wait("please run", "thread-1")

        self.assertIs(result, expected)
        start_turn.assert_not_called()
        steer_or_start.assert_called_once_with(
            app_server_transport.DEFAULT_CLIENT,
            "please run",
            "thread-1",
            bridge_module=bridge_module,
            confirm_timeout_sec=25.0,
        )

    def test_default_no_wait_delivery_uses_configured_confirm_timeout(self) -> None:
        bridge_module = cast(app_server_delivery.BridgeModule, cast(object, type("FakeBridge", (), {})()))

        def legacy_prompt(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
            return 0, f"legacy:{prompt}:{target_thread_id}"

        def watch(steering_result: FakeSteeringResult, relay: discord_stream.DiscordAskRelay) -> tuple[int, str]:
            _ = steering_result
            _ = relay
            return 0, "watched"

        def bridge_stream(argv: list[str], on_line: Callable[[str], None]) -> tuple[int, str]:
            _ = argv
            _ = on_line
            return 0, "legacy stream"

        def discard_log(message: str) -> None:
            _ = message

        with (
            mock.patch.dict("os.environ", {"DISCORD_STEERING_DELIVERY_CONFIRM_TIMEOUT_SECONDS": "42"}),
            mock.patch.object(
                discord_app_server,
                "run_prompt_no_wait",
                return_value=(0, "delivered"),
            ) as run_prompt,
        ):
            deps = prompt_transport_factory.make_prompt_transport_deps(
                bridge_module=bridge_module,
                app_server_transport_enabled=lambda: True,
                run_legacy_prompt_no_wait=legacy_prompt,
                make_steering_prompt_result=make_app_steering_result,
                run_watch_stream=watch,
                run_bridge_command_stream=bridge_stream,
                ui_fallback_lock=Lock(),
                log=discard_log,
            )

            result = deps.run_resident_prompt_no_wait("please run", "thread-1")

        self.assertEqual(result, (0, "delivered"))
        run_prompt.assert_called_once_with(
            "please run",
            "thread-1",
            transport_module=app_server_transport,
            bridge_module=bridge_module,
            client=app_server_transport.DEFAULT_CLIENT,
            confirm_timeout_sec=42.0,
        )


if __name__ == "__main__":
    _ = unittest.main()
