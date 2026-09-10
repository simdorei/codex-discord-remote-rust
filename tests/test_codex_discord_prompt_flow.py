from __future__ import annotations

from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
import unittest

import codex_discord_prompt_flow as prompt_flow
import codex_discord_prompt_pending_delivery as pending_delivery


@dataclass(frozen=True, slots=True)
class FakeChannel:
    messages: list[tuple[str, str | None]] = field(default_factory=list)


@dataclass(frozen=True, slots=True)
class FakeRelay:
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False
    suppressed_after_steering: bool = False
    relay_generation: int = 0


@dataclass(frozen=True, slots=True)
class PromptFlowFixture:
    warning: str = ""
    ask_start_messages: list[tuple[str, bool]] = field(default_factory=list)

    def build_context_warning(self, target_thread_id: str | None) -> str:
        _ = target_thread_id
        return self.warning

    def build_ask_start_message(self, prompt: str, *, queued: bool = False) -> str:
        self.ask_start_messages.append((prompt, queued))
        label = "Queued" if queued else "In progress"
        return f"{label}\nmessage: {prompt}"

    async def send_chunks(
        self,
        channel: FakeChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        channel.messages.append((content, context))

    def build(self) -> prompt_flow.PromptFlowPreambleDeps[FakeChannel]:
        return prompt_flow.PromptFlowPreambleDeps(
            build_context_warning=self.build_context_warning,
            build_ask_start_message=self.build_ask_start_message,
            send_chunks=self.send_chunks,
        )


class PromptFlowPreambleTests(unittest.IsolatedAsyncioTestCase):
    async def test_empty_warning_sends_only_ask_start(self) -> None:
        fixture = PromptFlowFixture()
        channel = FakeChannel()

        await prompt_flow.send_prompt_flow_preamble(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertEqual(channel.messages, [("In progress\nmessage: please run", "ask_start")])
        self.assertEqual(fixture.ask_start_messages, [("please run", False)])

    async def test_warning_is_sent_before_ask_start(self) -> None:
        fixture = PromptFlowFixture(warning="context warning")
        channel = FakeChannel()

        await prompt_flow.send_prompt_flow_preamble(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertEqual(
            channel.messages,
            [("context warning", None), ("In progress\nmessage: please run", "ask_start")],
        )

    async def test_queued_prompt_uses_queued_start_message(self) -> None:
        fixture = PromptFlowFixture()
        channel = FakeChannel()

        await prompt_flow.send_prompt_flow_preamble(
            channel,
            "please queue",
            "thread-1",
            queued=True,
            deps=fixture.build(),
        )

        self.assertEqual(channel.messages, [("Queued\nmessage: please queue", "ask_start")])
        self.assertEqual(fixture.ask_start_messages, [("please queue", True)])


class InitialStreamFlowTests(unittest.IsolatedAsyncioTestCase):
    async def test_returns_unhandled_stream_result(self) -> None:
        events: list[str] = []

        def make_relay(
            channel: str,
            *,
            target_thread_id: str | None,
            target_ref: str,
            started_at: float,
            delegate_to_session_mirror: bool,
        ) -> FakeRelay:
            events.append(f"relay:{channel}:{target_thread_id}:{target_ref}:{started_at}:{delegate_to_session_mirror}")
            return FakeRelay()

        @asynccontextmanager
        async def channel_typing(channel: str, *, context: str) -> AsyncGenerator[None, None]:
            events.append(f"typing:{channel}:{context}")
            yield

        async def run_ask_stream(prompt: str, relay: FakeRelay, *, target_thread_id: str | None) -> tuple[int, str]:
            events.append(f"stream:{prompt}:{target_thread_id}:{relay.sent_live}")
            return 0, "ok"

        async def send_chunks(channel: str, content: str, *, context: str | None = None) -> None:
            events.append(f"send:{channel}:{content}:{context}")

        result = await prompt_flow.run_initial_stream_flow(
            "channel",
            prompt="prompt",
            target_thread_id="thread-1",
            target_ref="project:1",
            delegate_to_session_mirror=True,
            deps=prompt_flow.make_initial_stream_flow_deps(
                monotonic=lambda: 2.0,
                make_relay=make_relay,
                channel_typing=channel_typing,
                run_ask_stream=run_ask_stream,
                is_discord_relay_stale=lambda target, generation: False,
                pending_delivery_deps=pending_delivery.AskStreamPendingDeliveryDeps(
                    is_delivery_confirmation_timeout=lambda output: False,
                    send_chunks=send_chunks,
                    format_log_text_len=lambda text: len(text or ""),
                    log=lambda message: events.append(f"log:{message}"),
                ),
                format_log_text_len=lambda text: len(text or ""),
                log=lambda message: events.append(f"log:{message}"),
            ),
        )

        self.assertFalse(result.handled)
        self.assertEqual((result.exit_code, result.output, result.started_at), (0, "ok", 2.0))
        self.assertEqual(result.relay, FakeRelay())
        self.assertIn("typing:channel:ask_stream", events)
