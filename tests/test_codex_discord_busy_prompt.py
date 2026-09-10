from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_busy_prompt as busy_prompt


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int
    bot: bool


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int


@dataclass(frozen=True, slots=True)
class FakeMessage:
    author: FakeAuthor
    channel: FakeChannel


class BusyPromptTests(unittest.IsolatedAsyncioTestCase):
    async def test_bot_authored_busy_message_auto_queues(self) -> None:
        # Given
        channel = FakeChannel(222)
        message = FakeMessage(FakeAuthor(1500506752234422322, bot=True), channel)
        enqueued: list[tuple[str, str | None, bool, bool, FakeMessage | None]] = []
        sent: list[tuple[FakeChannel, str, str]] = []
        busy_choices: list[str] = []
        logs: list[str] = []

        async def enqueue_thread_ask(
            channel: FakeChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: FakeMessage | None = None,
        ) -> int:
            self.assertEqual(channel.id, 222)
            enqueued.append((prompt, target_thread_id, queued, ack_sent, source_message))
            return 2

        async def send_busy_choice_message(
            channel: FakeChannel,
            source_message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None,
            allow_steer: bool,
            reason: str,
        ) -> bool:
            _ = (channel, source_message, target_thread_id, allow_steer)
            busy_choices.append(f"{reason}:{prompt}")
            return True

        async def send_chunks(target: FakeChannel, text: str, *, context: str = "send_chunks") -> int:
            sent.append((target, text, context))
            return len(text)

        # When
        await busy_prompt.handle_busy_prompt(
            channel,
            message,
            "continue from handoff",
            target_thread_id="thread-1",
            allow_steer=True,
            reason="same_thread_runner_busy",
            deps=busy_prompt.BusyPromptDeps(
                enqueue_thread_ask=enqueue_thread_ask,
                send_busy_choice_message=send_busy_choice_message,
                send_chunks=send_chunks,
                format_log_text_len=lambda text: str(len(text or "")),
                log=logs.append,
            ),
        )

        # Then
        self.assertEqual(enqueued, [("continue from handoff", "thread-1", True, False, message)])
        self.assertEqual(busy_choices, [])
        self.assertEqual(sent, [(channel, "Queued bot-authored message after the current Codex turn.", "bot_busy_prompt_auto_queued")])
        self.assertEqual(
            logs,
            [
                "bot_busy_prompt_auto_queued reason=same_thread_runner_busy "
                + "target=thread-1 position=2 prompt_len=21"
            ],
        )

    async def test_human_busy_message_keeps_busy_choice(self) -> None:
        # Given
        channel = FakeChannel(222)
        message = FakeMessage(FakeAuthor(242286902982606848, bot=False), channel)
        enqueued: list[str] = []
        sent: list[str] = []
        busy_choices: list[tuple[str, str | None, bool]] = []

        async def enqueue_thread_ask(
            channel: FakeChannel,
            prompt: str,
            target_thread_id: str | None,
            *,
            queued: bool = False,
            ack_sent: bool = False,
            source_message: FakeMessage | None = None,
        ) -> int:
            _ = (channel, target_thread_id, queued, ack_sent, source_message)
            enqueued.append(prompt)
            return 1

        async def send_busy_choice_message(
            channel: FakeChannel,
            source_message: FakeMessage,
            prompt: str,
            *,
            target_thread_id: str | None,
            allow_steer: bool,
            reason: str,
        ) -> bool:
            self.assertEqual(channel.id, 222)
            self.assertIs(source_message, message)
            _ = prompt
            busy_choices.append((reason, target_thread_id, allow_steer))
            return True

        async def send_chunks(target: FakeChannel, text: str, *, context: str = "send_chunks") -> int:
            _ = (target, context)
            sent.append(text)
            return len(text)

        # When
        await busy_prompt.handle_busy_prompt(
            channel,
            message,
            "please queue",
            target_thread_id="thread-1",
            allow_steer=True,
            reason="same_thread_runner_busy",
            deps=busy_prompt.BusyPromptDeps(
                enqueue_thread_ask=enqueue_thread_ask,
                send_busy_choice_message=send_busy_choice_message,
                send_chunks=send_chunks,
                format_log_text_len=lambda text: str(len(text or "")),
                log=lambda text: None,
            ),
        )

        # Then
        self.assertEqual(enqueued, [])
        self.assertEqual(sent, [])
        self.assertEqual(busy_choices, [("same_thread_runner_busy", "thread-1", True)])


if __name__ == "__main__":
    _ = unittest.main()
