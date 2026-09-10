from __future__ import annotations

import asyncio
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from unittest import mock

import codex_discord_delivery as delivery
import codex_discord_delivery_runtime as delivery_runtime
import codex_discord_delivery_interactions as delivery_interactions
from codex_discord_text import DISCORD_MAX_LEN


class SentMessage:
    def __init__(self, message_id: int) -> None:
        self.id: int = message_id


class FakeTarget:
    def __init__(self, *, channel_id: int = 123, failures: int = 0) -> None:
        self.id: int = channel_id
        self.failures: int = failures
        self.messages: list[tuple[str, dict[str, object]]] = []

    async def send(self, content: str, **kwargs: object) -> SentMessage:
        if self.failures > 0:
            self.failures -= 1
            raise RuntimeError("transient send failure")
        self.messages.append((content, kwargs))
        return SentMessage(len(self.messages))


class BlockingTarget(FakeTarget):
    def __init__(self, *, fail_first_send: bool = False) -> None:
        super().__init__()
        self.fail_first_send = fail_first_send
        self.first_send_started = asyncio.Event()
        self.release_first_send = asyncio.Event()
        self.send_calls = 0

    async def send(self, content: str, **kwargs: object) -> SentMessage:
        self.send_calls += 1
        if self.send_calls == 1:
            self.first_send_started.set()
            await self.release_first_send.wait()
            if self.fail_first_send:
                raise RuntimeError("first path failed")
        return await super().send(content, **kwargs)


class FakeInteractionResponse:
    def __init__(self) -> None:
        self.messages: list[tuple[str, bool]] = []

    async def send_message(self, content: str, *, ephemeral: bool = False) -> None:
        self.messages.append((content, ephemeral))


class FakeInteractionCommand:
    def __init__(self, name: str) -> None:
        self.name: str = name


class FakeInteraction:
    def __init__(self, *, command_name: str = "ask", channel_id: int = 123) -> None:
        self.command: FakeInteractionCommand = FakeInteractionCommand(command_name)
        self.channel_id: int = channel_id
        self.response: FakeInteractionResponse = FakeInteractionResponse()


class DiscordDeliveryTests(unittest.IsolatedAsyncioTestCase):
    def test_read_attachment_source_bytes_supports_data_url(self) -> None:
        payload = delivery_runtime.read_attachment_source_bytes("data:text/plain;base64,aGVsbG8=")

        self.assertEqual(payload, b"hello")

    def test_read_attachment_source_bytes_rejects_local_file_path(self) -> None:
        with self.assertRaises(delivery_runtime.AttachmentDataUrlError):
            _ = delivery_runtime.read_attachment_source_bytes("C:/tmp/report.txt")

    def test_read_attachment_source_bytes_supports_local_output_file_path(self) -> None:
        with TemporaryDirectory() as temp_dir:
            output_root = Path(temp_dir)
            attachment_path = output_root / "report.txt"
            _ = attachment_path.write_bytes(b"hello file")

            with mock.patch.object(
                delivery_runtime,
                "CODEX_SESSION_MIRROR_ATTACHMENT_DIR",
                output_root,
            ):
                payload = delivery_runtime.read_attachment_source_bytes(str(attachment_path))

        self.assertEqual(payload, b"hello file")

    def test_interaction_helpers_are_reexported_from_delivery_module(self) -> None:
        self.assertIs(
            delivery.send_interaction_response_tracked,
            delivery_interactions.send_interaction_response_tracked,
        )
        self.assertIs(
            delivery.send_interaction_not_allowed,
            delivery_interactions.send_interaction_not_allowed,
        )

    async def test_send_chunks_marks_and_retries_transient_failure(self) -> None:
        logs: list[str] = []
        state = delivery.DiscordDeliveryState(retry_delays_seconds=(0.0,))
        target = FakeTarget(failures=1)

        sent = await delivery.send_chunks(
            state,
            target,
            "retry me",
            log_func=logs.append,
            context="unit_retry",
        )

        self.assertEqual(sent, 1)
        self.assertEqual(target.messages, [("retry me", {})])
        self.assertFalse(state.active_deliveries)
        self.assertTrue(any("discord_delivery_retry" in line for line in logs))
        self.assertTrue(any("discord_delivery_sent" in line for line in logs))

    async def test_send_chunks_suppresses_session_mirror_copy_after_direct_reply(self) -> None:
        logs: list[str] = []
        state = delivery.DiscordDeliveryState()
        target = FakeTarget()

        direct_count = await delivery.send_chunks(
            state,
            target,
            "same final reply",
            log_func=logs.append,
            context="send_chunks",
        )
        mirror_count = await delivery.send_chunks(
            state,
            target,
            "same final reply",
            log_func=logs.append,
            context="session_mirror:final:thread-1",
        )

        self.assertEqual((direct_count, mirror_count), (1, 0))
        self.assertEqual(target.messages, [("same final reply", {})])
        self.assertTrue(any("discord_delivery_duplicate_suppressed" in line for line in logs))

    async def test_send_chunks_suppresses_direct_copy_after_session_mirror_reply(self) -> None:
        state = delivery.DiscordDeliveryState()
        target = FakeTarget()

        mirror_count = await delivery.send_chunks(
            state,
            target,
            "same commentary",
            log_func=lambda _line: None,
            context="session_mirror:commentary:thread-1",
        )
        direct_count = await delivery.send_chunks(
            state,
            target,
            "same commentary",
            log_func=lambda _line: None,
            context="send_chunks",
        )

        self.assertEqual((mirror_count, direct_count), (1, 0))
        self.assertEqual(target.messages, [("same commentary", {})])

    async def test_send_chunks_waits_for_concurrent_cross_path_delivery(self) -> None:
        state = delivery.DiscordDeliveryState()
        target = BlockingTarget()

        direct_task = asyncio.create_task(
            delivery.send_chunks(
                state,
                target,
                "concurrent final reply",
                log_func=lambda _line: None,
                context="send_chunks",
            )
        )
        await target.first_send_started.wait()
        mirror_task = asyncio.create_task(
            delivery.send_chunks(
                state,
                target,
                "concurrent final reply",
                log_func=lambda _line: None,
                context="session_mirror:final:thread-1",
            )
        )
        await asyncio.sleep(0)

        self.assertFalse(mirror_task.done())
        target.release_first_send.set()
        direct_count, mirror_count = await asyncio.gather(direct_task, mirror_task)

        self.assertEqual((direct_count, mirror_count), (1, 0))
        self.assertEqual(target.messages, [("concurrent final reply", {})])

    async def test_send_chunks_cross_path_waiter_takes_over_after_failure(self) -> None:
        state = delivery.DiscordDeliveryState(retry_delays_seconds=())
        target = BlockingTarget(fail_first_send=True)

        direct_task = asyncio.create_task(
            delivery.send_chunks(
                state,
                target,
                "recover failed delivery",
                log_func=lambda _line: None,
                context="send_chunks",
            )
        )
        await target.first_send_started.wait()
        mirror_task = asyncio.create_task(
            delivery.send_chunks(
                state,
                target,
                "recover failed delivery",
                log_func=lambda _line: None,
                context="session_mirror:final:thread-1",
            )
        )
        await asyncio.sleep(0)
        target.release_first_send.set()

        with self.assertRaisesRegex(RuntimeError, "first path failed"):
            _ = await direct_task
        mirror_count = await mirror_task

        self.assertEqual(mirror_count, 1)
        self.assertEqual(target.messages, [("recover failed delivery", {})])

    async def test_send_chunks_keeps_repeated_messages_from_the_same_path(self) -> None:
        state = delivery.DiscordDeliveryState()
        target = FakeTarget()

        first_count = await delivery.send_chunks(
            state,
            target,
            "intentional repeat",
            log_func=lambda _line: None,
            context="send_chunks",
        )
        second_count = await delivery.send_chunks(
            state,
            target,
            "intentional repeat",
            log_func=lambda _line: None,
            context="send_chunks",
        )

        self.assertEqual((first_count, second_count), (1, 1))
        self.assertEqual(len(target.messages), 2)

    async def test_send_chunks_can_disable_cross_path_deduplication(self) -> None:
        state = delivery.DiscordDeliveryState(cross_path_dedupe_seconds=0.0)
        target = FakeTarget()

        _ = await delivery.send_chunks(
            state,
            target,
            "repeat outside dedupe",
            log_func=lambda _line: None,
            context="send_chunks",
        )
        _ = await delivery.send_chunks(
            state,
            target,
            "repeat outside dedupe",
            log_func=lambda _line: None,
            context="session_mirror:final:thread-1",
        )

        self.assertEqual(len(target.messages), 2)

    async def test_stopping_rejects_new_delivery_but_allows_restart_notice(self) -> None:
        logs: list[str] = []
        state = delivery.DiscordDeliveryState()
        target = FakeTarget()
        delivery.set_discord_delivery_stopping(state, "unit", log_func=logs.append)

        with self.assertRaises(delivery.DiscordDeliveryRejected):
            _ = await delivery.send_chunks(state, target, "blocked", log_func=logs.append)

        await delivery.send_discord_restarting_notice(state, target, log_func=logs.append)

        self.assertEqual(len(target.messages), 1)
        self.assertIn("Discord bot is restarting", target.messages[0][0])
        self.assertTrue(any("discord_delivery_rejected" in line for line in logs))
        self.assertTrue(any("context=restart_notice" in line for line in logs))

    async def test_interaction_response_rejects_new_delivery_while_stopping(self) -> None:
        logs: list[str] = []
        state = delivery.DiscordDeliveryState(stopping=True)
        interaction = FakeInteraction()

        with self.assertRaises(delivery.DiscordDeliveryRejected):
            await delivery.send_interaction_response_tracked(
                state,
                interaction,
                "blocked",
                log_func=logs.append,
                ephemeral=True,
            )

        self.assertEqual(interaction.response.messages, [])
        self.assertTrue(any("context=response:" in line for line in logs))

    async def test_wait_for_drain_waits_for_active_delivery(self) -> None:
        logs: list[str] = []
        state = delivery.DiscordDeliveryState()
        token = delivery.begin_discord_delivery(state, "unit", log_func=logs.append)

        async def release() -> None:
            await asyncio.sleep(0.02)
            delivery.end_discord_delivery(state, token)

        release_task = asyncio.create_task(release())
        drained = await delivery.wait_for_discord_delivery_drain(
            state,
            timeout_seconds=1.0,
            reason="unit",
            log_func=logs.append,
        )
        await release_task

        self.assertTrue(drained)
        self.assertTrue(any("discord_delivery_drain_done reason=unit" in line for line in logs))

    def test_split_delivery_chunks_adds_markers_within_discord_limit(self) -> None:
        state = delivery.DiscordDeliveryState(chunk_markers_enabled=True)

        chunks = delivery.split_delivery_chunks("x" * (DISCORD_MAX_LEN + 200), state=state)

        self.assertGreater(len(chunks), 1)
        self.assertTrue(chunks[0].startswith("[1/"))
        self.assertTrue(all(len(chunk) <= DISCORD_MAX_LEN for chunk in chunks))
