from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_session_mirror_item_sender as item_sender


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int


SendCall = tuple[FakeChannel, item_sender.SessionMirrorItem, str, str]


class SessionMirrorItemSenderTests(unittest.IsolatedAsyncioTestCase):
    async def test_unclaimed_final_item_sends_claims_and_marks_terminal(self) -> None:
        channel = FakeChannel(channel_id=123)
        sends: list[SendCall] = []
        claims: list[tuple[str, str]] = []

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            self.assertEqual((digest, codex_thread_id), ("digest-1", "thread-1"))
            return False

        async def send_item(
            channel: FakeChannel,
            item: item_sender.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            sends.append((channel, item, target_thread_id, target_ref))

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            claims.append((digest, codex_thread_id))
            return True

        result = await item_sender.send_unclaimed_session_mirror_items(
            channel,
            [{"kind": "final", "text": "done", "digest": "digest-1"}],
            codex_thread_id="thread-1",
            target_ref="",
            deps=item_sender.SessionMirrorItemSenderDeps(
                has_session_mirror_event=has_event,
                send_session_mirror_item=send_item,
                claim_session_mirror_event=claim_event,
            ),
        )

        self.assertEqual(result, item_sender.SessionMirrorItemSendResult(sent_count=1, terminal_sent=True))
        self.assertEqual(sends, [(channel, {"kind": "final", "text": "done", "digest": "digest-1"}, "thread-1", "thread-1")])
        self.assertEqual(claims, [("digest-1", "thread-1")])

    async def test_already_claimed_final_digest_is_skipped_without_terminal(self) -> None:
        channel = FakeChannel(channel_id=123)
        sends: list[SendCall] = []
        claims: list[tuple[str, str]] = []

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            self.assertEqual((digest, codex_thread_id), ("digest-1", "thread-1"))
            return True

        async def send_item(
            channel: FakeChannel,
            item: item_sender.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            sends.append((channel, item, target_thread_id, target_ref))

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            claims.append((digest, codex_thread_id))
            return True

        result = await item_sender.send_unclaimed_session_mirror_items(
            channel,
            [{"kind": "final", "text": "done", "digest": "digest-1"}],
            codex_thread_id="thread-1",
            target_ref="project:1",
            deps=item_sender.SessionMirrorItemSenderDeps(
                has_session_mirror_event=has_event,
                send_session_mirror_item=send_item,
                claim_session_mirror_event=claim_event,
            ),
        )

        self.assertEqual(result, item_sender.SessionMirrorItemSendResult(sent_count=0, terminal_sent=False))
        self.assertEqual(sends, [])
        self.assertEqual(claims, [])

    async def test_digest_free_item_sends_without_claim(self) -> None:
        channel = FakeChannel(channel_id=123)
        sends: list[SendCall] = []
        claims: list[tuple[str, str]] = []

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            raise AssertionError(f"digest-free item should not check claims: {digest} {codex_thread_id}")

        async def send_item(
            channel: FakeChannel,
            item: item_sender.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            sends.append((channel, item, target_thread_id, target_ref))

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            claims.append((digest, codex_thread_id))
            return True

        result = await item_sender.send_unclaimed_session_mirror_items(
            channel,
            [{"kind": "delta", "text": "body"}],
            codex_thread_id="thread-1",
            target_ref="project:1",
            deps=item_sender.SessionMirrorItemSenderDeps(
                has_session_mirror_event=has_event,
                send_session_mirror_item=send_item,
                claim_session_mirror_event=claim_event,
            ),
        )

        self.assertEqual(result, item_sender.SessionMirrorItemSendResult(sent_count=1, terminal_sent=False))
        self.assertEqual(sends, [(channel, {"kind": "delta", "text": "body"}, "thread-1", "project:1")])
        self.assertEqual(claims, [])

    async def test_aborted_and_failed_items_do_not_deactivate_the_session_target(self) -> None:
        channel = FakeChannel(channel_id=123)

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            _ = (digest, codex_thread_id)
            return False

        async def send_item(
            channel: FakeChannel,
            item: item_sender.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            _ = (channel, item, target_thread_id, target_ref)

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            _ = (digest, codex_thread_id)
            return True

        result = await item_sender.send_unclaimed_session_mirror_items(
            channel,
            [
                {"kind": "aborted", "text": "stopped", "digest": "abort-1"},
                {"kind": "failed", "text": "failed", "digest": "failed-1"},
            ],
            codex_thread_id="thread-1",
            target_ref="project:1",
            deps=item_sender.SessionMirrorItemSenderDeps(
                has_session_mirror_event=has_event,
                send_session_mirror_item=send_item,
                claim_session_mirror_event=claim_event,
            ),
        )

        self.assertEqual(result, item_sender.SessionMirrorItemSendResult(sent_count=2, terminal_sent=False))

    async def test_send_failure_propagates_before_claiming(self) -> None:
        channel = FakeChannel(channel_id=123)
        claims: list[tuple[str, str]] = []

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            self.assertEqual((digest, codex_thread_id), ("digest-1", "thread-1"))
            return False

        async def send_item(
            channel: FakeChannel,
            item: item_sender.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            _ = (channel, item, target_thread_id, target_ref)
            raise RuntimeError("send failed")

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            claims.append((digest, codex_thread_id))
            return True

        with self.assertRaisesRegex(RuntimeError, "send failed"):
            _ = await item_sender.send_unclaimed_session_mirror_items(
                channel,
                [{"kind": "final", "text": "done", "digest": "digest-1"}],
                codex_thread_id="thread-1",
                target_ref="project:1",
                deps=item_sender.SessionMirrorItemSenderDeps(
                    has_session_mirror_event=has_event,
                    send_session_mirror_item=send_item,
                    claim_session_mirror_event=claim_event,
                ),
            )

        self.assertEqual(claims, [])
