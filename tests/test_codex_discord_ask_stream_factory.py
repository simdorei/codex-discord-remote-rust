from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable
from dataclasses import dataclass, field
from typing import Protocol, cast
import unittest
from unittest.mock import patch

import codex_discord_ask_stream_factory as factory
import codex_discord_bot as bot
from codex_discord_prompt_busy_result import RecentOffsets


@dataclass(frozen=True, slots=True)
class FakeChannel:
    name: str


@dataclass(frozen=True, slots=True)
class FakeRelay(bot.DiscordAskRelay):
    name: str


@dataclass(frozen=True, slots=True)
class RelayCall:
    loop: asyncio.AbstractEventLoop
    channel: FakeChannel
    target_thread_id: str | None
    target_ref: str
    suppress_after_steering_since: float
    send_commentary_blocks: bool | None
    send_final_blocks: bool


class BotMakeDiscordAskRelay(Protocol):
    def __call__(
        self,
        channel: FakeChannel,
        *,
        target_thread_id: str | None,
        target_ref: str,
        started_at: float,
        delegate_to_session_mirror: bool,
    ) -> bot.DiscordAskRelay: ...


class BotRunAskStreamInThread(Protocol):
    def __call__(
        self,
        prompt: str,
        relay: FakeRelay,
        *,
        target_thread_id: str | None,
    ) -> Awaitable[tuple[int, str]]: ...


@dataclass(slots=True)  # noqa: MUTABLE_OK
class FactoryFixture:
    relay_calls: list[RelayCall] = field(default_factory=list)
    stream_calls: list[tuple[str, FakeRelay, str | None]] = field(default_factory=list)
    chunk_calls: list[tuple[FakeChannel, str, str | None]] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)

    def make_relay(
        self,
        loop: asyncio.AbstractEventLoop,
        channel: FakeChannel,
        target_thread_id: str | None,
        target_ref: str,
        *,
        suppress_after_steering_since: float,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> FakeRelay:
        self.relay_calls.append(
            RelayCall(
                loop=loop,
                channel=channel,
                target_thread_id=target_thread_id,
                target_ref=target_ref,
                suppress_after_steering_since=suppress_after_steering_since,
                send_commentary_blocks=send_commentary_blocks,
                send_final_blocks=send_final_blocks,
            )
        )
        return FakeRelay("relay")

    def run_stream(self, prompt: str, relay: FakeRelay, *, target_thread_id: str | None) -> tuple[int, str]:
        self.stream_calls.append((prompt, relay, target_thread_id))
        return 19, "stream-output"

    async def send_chunks(
        self,
        channel: FakeChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        self.chunk_calls.append((channel, content, context))

    def is_delivery_confirmation_timeout(self, output: str) -> bool:
        return output == "delivery-pending"

    async def handle_busy_prompt(
        self,
        channel: FakeChannel,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str,
        recent_offsets: RecentOffsets,
        transport_output: str,
        delegate_to_session_mirror: bool,
    ) -> bool:
        _ = channel, prompt, target_thread_id, target_ref, recent_offsets, transport_output, delegate_to_session_mirror
        return True

    async def wait_busy_settle(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        recent_offsets: RecentOffsets,
    ) -> None:
        _ = prompt, target_thread_id, recent_offsets

    def mark_steering_handoff(self, target_thread_id: str) -> None:
        _ = target_thread_id

    async def send_app_menu(
        self,
        channel: FakeChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        _ = channel, target_thread_id, output, reason
        return True

    def format_log_text_len(self, text: str | None) -> int:
        return len(text or "")


class AskStreamFactoryTests(unittest.IsolatedAsyncioTestCase):
    async def test_pending_deps_forward_supplied_functions(self) -> None:
        # Given
        fixture = FactoryFixture()
        channel = FakeChannel("pending")
        is_delivery_confirmation_timeout = fixture.is_delivery_confirmation_timeout
        send_chunks = fixture.send_chunks
        format_log_text_len = fixture.format_log_text_len
        log = fixture.logs.append

        # When
        deps = factory.make_ask_stream_pending_delivery_deps(
            is_delivery_confirmation_timeout=is_delivery_confirmation_timeout,
            send_chunks=send_chunks,
            format_log_text_len=format_log_text_len,
            log=log,
        )

        # Then
        self.assertIs(deps.is_delivery_confirmation_timeout, is_delivery_confirmation_timeout)
        self.assertIs(deps.send_chunks, send_chunks)
        self.assertIs(deps.format_log_text_len, format_log_text_len)
        self.assertIs(deps.log, log)
        self.assertTrue(deps.is_delivery_confirmation_timeout("delivery-pending"))
        await deps.send_chunks(channel, "body", context="ctx")
        self.assertEqual(deps.format_log_text_len("abc"), 3)
        deps.log("line")
        self.assertEqual(fixture.chunk_calls, [(channel, "body", "ctx")])
        self.assertEqual(fixture.logs, ["line"])

    async def test_relay_factory_uses_running_loop_and_delegate_flags(self) -> None:
        # Given
        fixture = FactoryFixture()
        channel = FakeChannel("relay")
        running_loop = asyncio.get_running_loop()

        # When
        relay = factory.make_discord_ask_relay(
            fixture.make_relay,
            channel,
            target_thread_id="thread-1",
            target_ref="project:1",
            started_at=12.5,
            delegate_to_session_mirror=True,
        )

        # Then
        self.assertEqual(relay, FakeRelay("relay"))
        self.assertEqual(
            fixture.relay_calls,
            [
                RelayCall(
                    loop=running_loop,
                    channel=channel,
                    target_thread_id="thread-1",
                    target_ref="project:1",
                    suppress_after_steering_since=12.5,
                    send_commentary_blocks=False,
                    send_final_blocks=False,
                )
            ],
        )

    async def test_relay_factory_keeps_commentary_default_when_not_delegating(self) -> None:
        # Given
        fixture = FactoryFixture()
        channel = FakeChannel("relay")

        # When
        _ = factory.make_discord_ask_relay(
            fixture.make_relay,
            channel,
            target_thread_id=None,
            target_ref="selected",
            started_at=20.0,
            delegate_to_session_mirror=False,
        )

        # Then
        self.assertEqual(fixture.relay_calls[0].send_commentary_blocks, None)
        self.assertTrue(fixture.relay_calls[0].send_final_blocks)

    async def test_run_ask_stream_in_thread_forwards_stream_runner(self) -> None:
        # Given
        fixture = FactoryFixture()
        relay = FakeRelay("thread-runner")

        # When
        result = await factory.run_ask_stream_in_thread(
            fixture.run_stream,
            "prompt text",
            relay,
            target_thread_id="thread-2",
        )

        # Then
        self.assertEqual(result, (19, "stream-output"))
        self.assertEqual(fixture.stream_calls, [("prompt text", relay, "thread-2")])

    async def test_busy_result_deps_forward_supplied_functions(self) -> None:
        # Given
        fixture = FactoryFixture()
        handle_busy_prompt = fixture.handle_busy_prompt
        wait_busy_settle = fixture.wait_busy_settle
        mark_steering_handoff = fixture.mark_steering_handoff
        send_app_menu = fixture.send_app_menu
        format_log_text_len = fixture.format_log_text_len
        log = fixture.logs.append

        # When
        deps = factory.make_ask_stream_busy_result_deps(
            handle_recorded_busy_transport_prompt=handle_busy_prompt,
            wait_for_mirrored_busy_delegation_settle=wait_busy_settle,
            mark_steering_handoff=mark_steering_handoff,
            send_codex_app_menu_if_available=send_app_menu,
            format_log_text_len=format_log_text_len,
            log=log,
        )

        # Then
        self.assertIs(deps.handle_recorded_busy_transport_prompt, handle_busy_prompt)
        self.assertIs(deps.wait_for_mirrored_busy_delegation_settle, wait_busy_settle)
        self.assertIs(deps.mark_steering_handoff, mark_steering_handoff)
        self.assertIs(deps.send_codex_app_menu_if_available, send_app_menu)
        self.assertIs(deps.format_log_text_len, format_log_text_len)
        self.assertIs(deps.log, log)

    async def test_bot_make_discord_ask_relay_uses_current_relay_factory(self) -> None:
        # Given
        fixture = FactoryFixture()
        channel = FakeChannel("bot-relay")

        # When
        with patch.object(bot, "DiscordAskRelay", fixture.make_relay):
            make_relay = cast(BotMakeDiscordAskRelay, getattr(bot, "_make_discord_ask_relay"))
            relay = make_relay(
                channel,
                target_thread_id="thread-4",
                target_ref="project:4",
                started_at=45.5,
                delegate_to_session_mirror=True,
            )

        # Then
        self.assertEqual(relay, FakeRelay("relay"))
        self.assertEqual(fixture.relay_calls[0].target_thread_id, "thread-4")
        self.assertEqual(fixture.relay_calls[0].target_ref, "project:4")
        self.assertEqual(fixture.relay_calls[0].suppress_after_steering_since, 45.5)
        self.assertEqual(fixture.relay_calls[0].send_commentary_blocks, False)
        self.assertEqual(fixture.relay_calls[0].send_final_blocks, False)

    async def test_bot_run_ask_stream_in_thread_uses_current_runner(self) -> None:
        # Given
        fixture = FactoryFixture()
        relay = FakeRelay("bot-runner")

        # When
        with patch.object(bot, "run_ask_stream", fixture.run_stream):
            run_in_thread = cast(BotRunAskStreamInThread, getattr(bot, "_run_ask_stream_in_thread"))
            result = await run_in_thread(
                "bot prompt",
                relay,
                target_thread_id="thread-5",
            )

        # Then
        self.assertEqual(result, (19, "stream-output"))
        self.assertEqual(fixture.stream_calls, [("bot prompt", relay, "thread-5")])


if __name__ == "__main__":
    _ = unittest.main()
