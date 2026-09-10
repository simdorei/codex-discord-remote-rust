from __future__ import annotations

from dataclasses import dataclass
import unittest

import codex_discord_session_mirror_delivery_flow as delivery_flow


@dataclass(frozen=True, slots=True)
class FakeChannel:
    channel_id: int


SendCall = tuple[FakeChannel, delivery_flow.SessionMirrorItem, str, str]


class SessionMirrorDeliveryFlowTests(unittest.IsolatedAsyncioTestCase):
    async def test_deliver_and_commit_sends_claims_updates_cursor_and_logs(self) -> None:
        channel = FakeChannel(channel_id=333)
        items: list[delivery_flow.SessionMirrorItem] = [
            {"kind": "final", "text": "done", "digest": "digest-1"}
        ]
        resolved_channels: list[int] = []
        sends: list[SendCall] = []
        claims: list[tuple[str, str]] = []
        updates: list[tuple[str, str, int]] = []
        released: list[str] = []
        logs: list[str] = []

        async def resolve_channel(discord_thread_id: int) -> FakeChannel | None:
            resolved_channels.append(discord_thread_id)
            return channel

        def resolve_target_ref(codex_thread_id: str) -> tuple[str | None, str]:
            return codex_thread_id, "project:1"

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            self.assertEqual((digest, codex_thread_id), ("digest-1", "thread-1"))
            return False

        async def send_item(
            channel: FakeChannel,
            item: delivery_flow.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            sends.append((channel, item, target_thread_id, target_ref))

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            claims.append((digest, codex_thread_id))
            return True

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def release_target(codex_thread_id: str) -> bool:
            released.append(codex_thread_id)
            return True

        delivered = await delivery_flow.deliver_and_commit_session_mirror_items(
            "thread-1",
            "session.jsonl",
            42,
            discord_thread_id=333,
            event_count=2,
            items=items,
            deps=delivery_flow.SessionMirrorDeliveryFlowDeps(
                resolve_session_mirror_channel=resolve_channel,
                resolve_target_ref=resolve_target_ref,
                has_session_mirror_event=has_event,
                send_session_mirror_item=send_item,
                claim_session_mirror_event=claim_event,
                update_session_mirror_cursor=update_cursor,
                release_session_mirror_output_target=release_target,
                log=logs.append,
            ),
        )

        self.assertTrue(delivered)
        self.assertEqual(resolved_channels, [333])
        self.assertEqual(sends, [(channel, items[0], "thread-1", "project:1")])
        self.assertEqual(claims, [("digest-1", "thread-1")])
        self.assertEqual(updates, [("thread-1", "session.jsonl", 42)])
        self.assertEqual(released, ["thread-1"])
        self.assertEqual(
            logs,
            ["session_mirror_sent target=thread-1 channel=333 events=2 items=1 cursor=42"],
        )

    async def test_missing_channel_returns_without_delivery_or_commit(self) -> None:
        items: list[delivery_flow.SessionMirrorItem] = [
            {"kind": "final", "text": "done", "digest": "digest-1"}
        ]
        resolved_channels: list[int] = []

        async def resolve_channel(discord_thread_id: int) -> FakeChannel | None:
            resolved_channels.append(discord_thread_id)
            return None

        def resolve_target_ref(codex_thread_id: str) -> tuple[str | None, str]:
            raise AssertionError(f"target ref should not resolve: {codex_thread_id}")

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            raise AssertionError(f"claim check should not run: {digest} {codex_thread_id}")

        async def send_item(
            channel: FakeChannel,
            item: delivery_flow.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            raise AssertionError(f"send should not run: {channel} {item} {target_thread_id} {target_ref}")

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            raise AssertionError(f"claim should not run: {digest} {codex_thread_id}")

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            raise AssertionError(f"cursor should not update: {codex_thread_id} {rollout_path} {cursor}")

        async def release_target(codex_thread_id: str) -> bool:
            raise AssertionError(f"target should not release: {codex_thread_id}")

        def log(message: str) -> None:
            raise AssertionError(f"log should not be written: {message}")

        delivered = await delivery_flow.deliver_and_commit_session_mirror_items(
            "thread-1",
            "session.jsonl",
            42,
            discord_thread_id=333,
            event_count=2,
            items=items,
            deps=delivery_flow.SessionMirrorDeliveryFlowDeps(
                resolve_session_mirror_channel=resolve_channel,
                resolve_target_ref=resolve_target_ref,
                has_session_mirror_event=has_event,
                send_session_mirror_item=send_item,
                claim_session_mirror_event=claim_event,
                update_session_mirror_cursor=update_cursor,
                release_session_mirror_output_target=release_target,
                log=log,
            ),
        )

        self.assertFalse(delivered)
        self.assertEqual(resolved_channels, [333])

    async def test_send_failure_propagates_without_claim_or_cursor_advance(self) -> None:
        channel = FakeChannel(channel_id=333)
        items: list[delivery_flow.SessionMirrorItem] = [
            {"kind": "final", "text": "done", "digest": "digest-1"}
        ]
        claim_checks: list[tuple[str, str]] = []
        claims: list[tuple[str, str]] = []
        updates: list[tuple[str, str, int]] = []
        released: list[str] = []
        logs: list[str] = []

        async def resolve_channel(discord_thread_id: int) -> FakeChannel | None:
            self.assertEqual(discord_thread_id, 333)
            return channel

        def resolve_target_ref(codex_thread_id: str) -> tuple[str | None, str]:
            return codex_thread_id, "project:1"

        async def has_event(digest: str, codex_thread_id: str) -> bool:
            claim_checks.append((digest, codex_thread_id))
            return False

        async def send_item(
            channel: FakeChannel,
            item: delivery_flow.SessionMirrorItem,
            *,
            target_thread_id: str,
            target_ref: str,
        ) -> None:
            _ = (channel, item, target_thread_id, target_ref)
            raise RuntimeError("send failed")

        async def claim_event(digest: str, codex_thread_id: str) -> bool:
            claims.append((digest, codex_thread_id))
            return True

        async def update_cursor(codex_thread_id: str, rollout_path: str, cursor: int) -> None:
            updates.append((codex_thread_id, rollout_path, cursor))

        async def release_target(codex_thread_id: str) -> bool:
            released.append(codex_thread_id)
            return True

        with self.assertRaisesRegex(RuntimeError, "send failed"):
            _ = await delivery_flow.deliver_and_commit_session_mirror_items(
                "thread-1",
                "session.jsonl",
                42,
                discord_thread_id=333,
                event_count=2,
                items=items,
                deps=delivery_flow.SessionMirrorDeliveryFlowDeps(
                    resolve_session_mirror_channel=resolve_channel,
                    resolve_target_ref=resolve_target_ref,
                    has_session_mirror_event=has_event,
                    send_session_mirror_item=send_item,
                    claim_session_mirror_event=claim_event,
                    update_session_mirror_cursor=update_cursor,
                    release_session_mirror_output_target=release_target,
                    log=logs.append,
                ),
            )

        self.assertEqual(claim_checks, [("digest-1", "thread-1")])
        self.assertEqual(claims, [])
        self.assertEqual(updates, [])
        self.assertEqual(released, [])
        self.assertEqual(logs, [])
