from __future__ import annotations

import unittest
from collections.abc import Callable
from dataclasses import dataclass

import codex_discord_commands as discord_commands
import codex_discord_prefix_dispatch as prefix_dispatch


@dataclass(frozen=True)
class FakeChannel:
    id: int = 123


@dataclass(frozen=True)
class FakeAuthor:
    id: int = 456


@dataclass(frozen=True)
class FakeGuild:
    id: int = 789


@dataclass(frozen=True)
class FakeMessage:
    channel: FakeChannel = FakeChannel()
    author: FakeAuthor = FakeAuthor()
    guild: FakeGuild | None = FakeGuild()


class Capture:
    def __init__(self) -> None:
        self.sent: list[tuple[int, str, str]] = []
        self.bridge_builds: list[tuple[str, str, int]] = []
        self.bridge_runs: list[tuple[int, tuple[str, ...], str]] = []
        self.doctor_messages: list[int] = []
        self.doctor_runs: list[int] = []
        self.handler_calls: list[tuple[int, str, str, int]] = []
        self.labels: list[str] = []


HandlerMaker = Callable[[int, bool], prefix_dispatch.PrefixHandler]


class PrefixDispatchTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        bridge_actions: dict[str, discord_commands.PrefixBridgeAction | None] | None = None,
        handler_results: tuple[bool, ...] = (),
        format_label: Callable[[str], str] | None = None,
    ) -> tuple[prefix_dispatch.PrefixDispatchDeps, Capture]:
        capture = Capture()

        async def send_chunks(
            target: prefix_dispatch.PrefixDispatchChannel,
            text: str,
            *,
            context: str = "send_chunks",
        ) -> int:
            capture.sent.append((target.id, context, text))
            return 1

        def build_bridge_action(
            command: str,
            arg: str,
            channel_id: int,
        ) -> discord_commands.PrefixBridgeAction | None:
            capture.bridge_builds.append((command, arg, channel_id))
            return None if bridge_actions is None else bridge_actions.get(command)

        async def run_bridge_action(
            target: prefix_dispatch.PrefixDispatchChannel,
            argv: list[str],
            title: str,
        ) -> tuple[int, str]:
            capture.bridge_runs.append((target.id, tuple(argv), title))
            return 0, "bridge ok"

        async def build_doctor_message(message: prefix_dispatch.PrefixDispatchMessage) -> str:
            capture.doctor_messages.append(message.channel.id)
            return f"doctor:{message.channel.id}"

        async def run_doctor_bridge(target: prefix_dispatch.PrefixDispatchChannel) -> tuple[int, str]:
            capture.doctor_runs.append(target.id)
            return 0, "doctor ok"

        def make_handler(index: int, result: bool) -> prefix_dispatch.PrefixHandler:
            async def handler(
                command: str,
                arg: str,
                message: prefix_dispatch.PrefixDispatchMessage,
            ) -> bool:
                capture.handler_calls.append((index, command, arg, message.channel.id))
                return result

            return handler

        def label(command: str) -> str:
            capture.labels.append(command)
            if format_label is not None:
                return format_label(command)
            return command

        handlers = tuple(make_handler(index, result) for index, result in enumerate(handler_results))
        deps = prefix_dispatch.PrefixDispatchDeps(
            send_chunks=send_chunks,
            build_help=lambda: "help text",
            build_bridge_action=build_bridge_action,
            run_bridge_action=run_bridge_action,
            build_doctor_message=build_doctor_message,
            run_doctor_bridge=run_doctor_bridge,
            format_command_label=label,
            handlers=handlers,
        )
        return deps, capture

    async def test_sends_help_for_empty_help_and_start(self) -> None:
        deps, capture = self.make_deps()
        message = FakeMessage()

        await prefix_dispatch.handle_prefix_command(message, "", deps=deps)
        await prefix_dispatch.handle_prefix_command(message, "help", deps=deps)
        await prefix_dispatch.handle_prefix_command(message, "start", deps=deps)

        self.assertEqual(capture.sent, [(123, "send_chunks", "help text")] * 3)
        self.assertEqual(capture.bridge_builds, [])
        self.assertEqual(capture.handler_calls, [])

    async def test_sends_bridge_usage_without_running_bridge(self) -> None:
        deps, capture = self.make_deps(
            bridge_actions={"use": discord_commands.PrefixBridgeAction(None, "Use", "Usage: !use <ref>")}
        )

        await prefix_dispatch.handle_prefix_command(FakeMessage(), "use", deps=deps)

        self.assertEqual(capture.bridge_builds, [("use", "", 123)])
        self.assertEqual(capture.sent, [(123, "prefix_bridge_usage", "Usage: !use <ref>")])
        self.assertEqual(capture.bridge_runs, [])

    async def test_runs_bridge_action_with_parsed_arguments(self) -> None:
        deps, capture = self.make_deps(
            bridge_actions={"list": discord_commands.PrefixBridgeAction(["list", "--limit", "2"], "List")}
        )

        await prefix_dispatch.handle_prefix_command(FakeMessage(), "list 2", deps=deps)

        self.assertEqual(capture.bridge_builds, [("list", "2", 123)])
        self.assertEqual(capture.bridge_runs, [(123, ("list", "--limit", "2"), "List")])
        self.assertEqual(capture.sent, [])

    async def test_doctor_sends_diagnostics_then_runs_bridge(self) -> None:
        deps, capture = self.make_deps()

        await prefix_dispatch.handle_prefix_command(FakeMessage(), "doctor", deps=deps)

        self.assertEqual(capture.doctor_messages, [123])
        self.assertEqual(capture.sent, [(123, "send_chunks", "doctor:123")])
        self.assertEqual(capture.doctor_runs, [123])
        self.assertEqual(capture.handler_calls, [])

    async def test_runs_downstream_handlers_in_order_until_handled(self) -> None:
        deps, capture = self.make_deps(handler_results=(False, True, True))

        await prefix_dispatch.handle_prefix_command(FakeMessage(), "queue now", deps=deps)

        self.assertEqual(capture.handler_calls, [(0, "queue", "now", 123), (1, "queue", "now", 123)])
        self.assertEqual(capture.sent, [])
        self.assertEqual(capture.labels, [])

    async def test_unknown_command_uses_bounded_label(self) -> None:
        deps, capture = self.make_deps(format_label=lambda command: f"bounded:{command}")

        await prefix_dispatch.handle_prefix_command(FakeMessage(), "unknown arg", deps=deps)

        self.assertEqual(capture.labels, ["unknown"])
        self.assertEqual(capture.sent, [(123, "prefix_unknown", "Unknown command: !bounded:unknown")])
        self.assertEqual(capture.bridge_runs, [])

if __name__ == "__main__":
    _ = unittest.main()
