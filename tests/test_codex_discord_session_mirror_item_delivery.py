from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_session_mirror_item_delivery as item_delivery


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int


InteractiveCall = tuple[FakeChannel, str, str, str, str, list[tuple[str, str]]]
ChunkCall = tuple[FakeChannel, str, str]
AttachmentCall = tuple[FakeChannel, str, str, str, str]


class SessionMirrorItemDeliveryTests(unittest.IsolatedAsyncioTestCase):
    async def test_interactive_item_sends_prompt_without_chunks(self) -> None:
        channel = FakeChannel(channel_id=123)
        interactive_calls: list[InteractiveCall] = []
        chunk_calls: list[ChunkCall] = []

        async def send_interactive_prompt(
            channel: FakeChannel,
            target_thread_id: str,
            target_ref: str,
            state: str,
            prompt: str,
            options: list[tuple[str, str]],
        ) -> None:
            interactive_calls.append((channel, target_thread_id, target_ref, state, prompt, options))

        async def send_chunks(channel: FakeChannel, content: str, *, context: str) -> None:
            chunk_calls.append((channel, content, context))

        async def send_attachment(
            channel: FakeChannel,
            content: str,
            attachment_url: str,
            filename: str,
            *,
            context: str,
        ) -> None:
            _ = (channel, content, attachment_url, filename, context)
            raise AssertionError("interactive item should not send attachment")

        deps: item_delivery.SessionMirrorItemDeliveryDeps[FakeChannel] = (
            item_delivery.SessionMirrorItemDeliveryDeps(
                parse_interactive_notice=lambda text: ("waiting-input", "Pick", [("a", "A")]),
                send_interactive_prompt=send_interactive_prompt,
                send_chunks=send_chunks,
                send_attachment=send_attachment,
                format_session_mirror_text=lambda item: "formatted",
            )
        )

        await item_delivery.send_session_mirror_item(
            channel,
            {"kind": "interactive", "text": "notice"},
            target_thread_id="thread-1",
            target_ref="project:1",
            deps=deps,
        )

        self.assertEqual(
            interactive_calls,
            [(channel, "thread-1", "project:1", "waiting-input", "Pick", [("a", "A")])],
        )
        self.assertEqual(chunk_calls, [])

    async def test_regular_item_sends_formatted_chunks_with_context(self) -> None:
        channel = FakeChannel(channel_id=123)
        interactive_calls: list[InteractiveCall] = []
        chunk_calls: list[ChunkCall] = []

        async def send_interactive_prompt(
            channel: FakeChannel,
            target_thread_id: str,
            target_ref: str,
            state: str,
            prompt: str,
            options: list[tuple[str, str]],
        ) -> None:
            interactive_calls.append((channel, target_thread_id, target_ref, state, prompt, options))

        async def send_chunks(channel: FakeChannel, content: str, *, context: str) -> None:
            chunk_calls.append((channel, content, context))

        async def send_attachment(
            channel: FakeChannel,
            content: str,
            attachment_url: str,
            filename: str,
            *,
            context: str,
        ) -> None:
            _ = (channel, content, attachment_url, filename, context)
            raise AssertionError("regular item should not send attachment")

        deps: item_delivery.SessionMirrorItemDeliveryDeps[FakeChannel] = (
            item_delivery.SessionMirrorItemDeliveryDeps(
                parse_interactive_notice=lambda text: ("", "", []),
                send_interactive_prompt=send_interactive_prompt,
                send_chunks=send_chunks,
                send_attachment=send_attachment,
                format_session_mirror_text=lambda item: f"formatted:{item['text']}",
            )
        )

        await item_delivery.send_session_mirror_item(
            channel,
            {"kind": "final", "text": "done"},
            target_thread_id="thread-1",
            target_ref="project:1",
            deps=deps,
        )

        self.assertEqual(interactive_calls, [])
        self.assertEqual(chunk_calls, [(channel, "formatted:done", "session_mirror:final:thread-1")])

    async def test_attachment_item_sends_attachment_without_chunks(self) -> None:
        channel = FakeChannel(channel_id=123)
        chunk_calls: list[ChunkCall] = []
        attachment_calls: list[AttachmentCall] = []

        async def send_interactive_prompt(
            channel: FakeChannel,
            target_thread_id: str,
            target_ref: str,
            state: str,
            prompt: str,
            options: list[tuple[str, str]],
        ) -> None:
            _ = (channel, target_thread_id, target_ref, state, prompt, options)
            raise AssertionError("image item should not send interactive prompt")

        async def send_chunks(channel: FakeChannel, content: str, *, context: str) -> None:
            chunk_calls.append((channel, content, context))

        async def send_attachment(
            channel: FakeChannel,
            content: str,
            attachment_url: str,
            filename: str,
            *,
            context: str,
        ) -> None:
            attachment_calls.append((channel, content, attachment_url, filename, context))

        deps: item_delivery.SessionMirrorItemDeliveryDeps[FakeChannel] = (
            item_delivery.SessionMirrorItemDeliveryDeps(
                parse_interactive_notice=lambda text: ("", "", []),
                send_interactive_prompt=send_interactive_prompt,
                send_chunks=send_chunks,
                send_attachment=send_attachment,
                format_session_mirror_text=lambda item: f"formatted:{item['text']}",
            )
        )

        await item_delivery.send_session_mirror_item(
            channel,
            {
                "kind": "image",
                "text": "Codex image output",
                "attachment_url": "data:image/png;base64,aGVsbG8=",
                "attachment_filename": "codex-image-output.png",
            },
            target_thread_id="thread-1",
            target_ref="project:1",
            deps=deps,
        )

        self.assertEqual(chunk_calls, [])
        self.assertEqual(
            attachment_calls,
            [
                (
                    channel,
                    "formatted:Codex image output",
                    "data:image/png;base64,aGVsbG8=",
                    "codex-image-output.png",
                    "session_mirror:image:thread-1",
                )
            ],
        )

    async def test_interactive_without_state_falls_back_to_chunks(self) -> None:
        channel = FakeChannel(channel_id=123)
        chunk_calls: list[ChunkCall] = []

        async def send_interactive_prompt(
            channel: FakeChannel,
            target_thread_id: str,
            target_ref: str,
            state: str,
            prompt: str,
            options: list[tuple[str, str]],
        ) -> None:
            _ = (channel, target_thread_id, target_ref, state, prompt, options)
            raise AssertionError("missing state should not send interactive prompt")

        async def send_chunks(channel: FakeChannel, content: str, *, context: str) -> None:
            chunk_calls.append((channel, content, context))

        async def send_attachment(
            channel: FakeChannel,
            content: str,
            attachment_url: str,
            filename: str,
            *,
            context: str,
        ) -> None:
            _ = (channel, content, attachment_url, filename, context)
            raise AssertionError("malformed interactive item should not send attachment")

        deps: item_delivery.SessionMirrorItemDeliveryDeps[FakeChannel] = (
            item_delivery.SessionMirrorItemDeliveryDeps(
                parse_interactive_notice=lambda text: ("", "", []),
                send_interactive_prompt=send_interactive_prompt,
                send_chunks=send_chunks,
                send_attachment=send_attachment,
                format_session_mirror_text=lambda item: "fallback",
            )
        )

        await item_delivery.send_session_mirror_item(
            channel,
            {"kind": "interactive", "text": "malformed"},
            target_thread_id="thread-1",
            target_ref="project:1",
            deps=deps,
        )

        self.assertEqual(chunk_calls, [(channel, "fallback", "session_mirror:interactive:thread-1")])

    async def test_blank_kind_uses_unknown_context(self) -> None:
        channel = FakeChannel(channel_id=123)
        chunk_calls: list[ChunkCall] = []

        async def send_interactive_prompt(
            channel: FakeChannel,
            target_thread_id: str,
            target_ref: str,
            state: str,
            prompt: str,
            options: list[tuple[str, str]],
        ) -> None:
            _ = (channel, target_thread_id, target_ref, state, prompt, options)
            raise AssertionError("plain item should not send interactive prompt")

        async def send_chunks(channel: FakeChannel, content: str, *, context: str) -> None:
            chunk_calls.append((channel, content, context))

        async def send_attachment(
            channel: FakeChannel,
            content: str,
            attachment_url: str,
            filename: str,
            *,
            context: str,
        ) -> None:
            _ = (channel, content, attachment_url, filename, context)
            raise AssertionError("plain item should not send attachment")

        deps: item_delivery.SessionMirrorItemDeliveryDeps[FakeChannel] = (
            item_delivery.SessionMirrorItemDeliveryDeps(
                parse_interactive_notice=lambda text: ("waiting-input", "ignored", []),
                send_interactive_prompt=send_interactive_prompt,
                send_chunks=send_chunks,
                send_attachment=send_attachment,
                format_session_mirror_text=lambda item: "blank-kind",
            )
        )

        await item_delivery.send_session_mirror_item(
            channel,
            {"text": "body"},
            target_thread_id="thread-1",
            target_ref="project:1",
            deps=deps,
        )

        self.assertEqual(chunk_calls, [(channel, "blank-kind", "session_mirror:unknown:thread-1")])
