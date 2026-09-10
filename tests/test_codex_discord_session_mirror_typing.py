from __future__ import annotations

import tempfile
import unittest
from collections.abc import Mapping
from pathlib import Path
from typing import cast, final
from unittest import mock

import codex_discord_bot_session_mirror_runtime as bot_session_mirror_runtime
import codex_discord_pending_approval_delivery as pending_approval_delivery
import codex_discord_session_mirror as session_mirror
import codex_discord_session_mirror_item_sender as item_sender
import codex_discord_session_mirror_target as session_mirror_target
from codex_app_server_transport_replies import JsonObject
from codex_session_events import JsonEvent


@final
class FakeThread:
    def __init__(self, rollout_path: str) -> None:
        self.rollout_path: str = rollout_path


@final
class FakeChannel:
    id: int = 222


async def release_output_target(_thread_id: str) -> bool:
    return True


class SessionMirrorTypingTests(unittest.IsolatedAsyncioTestCase):
    async def test_pending_mcp_elicitation_is_sent_instead_of_only_typing(self) -> None:
        sent_items: list[dict[str, str]] = []
        claimed: list[tuple[str, str]] = []
        runtime_deps = mock.Mock()
        pending_request: JsonObject = {
            "id": "elicitation-1",
            "method": "mcpServer/elicitation/request",
            "params": {
                "threadId": "thread-1",
                "mode": "url",
                "message": "Install Google Drive and continue.",
                "url": "codex://plugins/install/?marketplace=openai-curated",
            },
        }
        def get_pending_request(thread_id: str) -> JsonObject | None:
            _ = thread_id
            return pending_request

        def claim_delivery(digest: str, thread_id: str) -> bool:
            claimed.append((digest, thread_id))
            return True

        delivery_deps: pending_approval_delivery.PendingApprovalDeliveryDeps[FakeChannel] = (
            pending_approval_delivery.PendingApprovalDeliveryDeps(
            parse_target=session_mirror.parse_session_mirror_target,
            get_pending_request=get_pending_request,
            has_delivered=lambda digest, thread_id: False,
            claim_delivery=claim_delivery,
            resolve_target_ref=lambda thread_id: (thread_id, thread_id),
            log=lambda message: None,
            )
        )

        async def deliver_pending_approval(
            owner: pending_approval_delivery.PendingApprovalDeliveryOwner[FakeChannel],
            target: session_mirror.SessionMirrorTargetMapping,
        ) -> bool:
            return await pending_approval_delivery.deliver_pending_approval(
                owner,
                target,
                deps=delivery_deps,
            )

        runtime_deps.deliver_pending_approval = deliver_pending_approval
        runtime = bot_session_mirror_runtime.SessionMirrorRuntime(
            cast(bot_session_mirror_runtime.SessionMirrorRuntimeDeps[FakeChannel], runtime_deps)
        )
        owner = mock.Mock()
        owner.resolve_session_mirror_channel = mock.AsyncMock(return_value=FakeChannel())

        async def send_item(channel: FakeChannel, item: dict[str, str], **kwargs: object) -> None:
            _ = channel
            _ = kwargs
            sent_items.append(item)

        owner.send_session_mirror_item = send_item

        with mock.patch.object(
            session_mirror_target,
            "mirror_session_target",
            new_callable=mock.AsyncMock,
        ) as mirror_target:
            await runtime.mirror_session_target(
                cast(bot_session_mirror_runtime.SessionMirrorOwner[FakeChannel], owner),
                {"codex_thread_id": "thread-1", "discord_thread_id": 222},
            )

        self.assertEqual(len(sent_items), 1, "Pending MCP approval must be visible in Discord.")
        self.assertEqual(sent_items[0]["kind"], "interactive")
        self.assertIn("[approval_required]", sent_items[0]["text"])
        self.assertIn("Install Google Drive and continue.", sent_items[0]["text"])
        self.assertIn("codex://plugins/install/", sent_items[0]["text"])
        self.assertEqual(claimed, [("app_server_request:elicitation-1", "thread-1")])
        mirror_target.assert_not_awaited()

    async def test_runtime_forwards_target_id_to_typing_pulse(self) -> None:
        typing_pulses: list[tuple[int, str, str]] = []
        runtime_deps_mock = mock.Mock()

        async def send_typing_pulse(channel: FakeChannel, target_thread_id: str, context: str) -> None:
            typing_pulses.append((channel.id, target_thread_id, context))

        runtime_deps_mock.configure_mock(
            deliver_pending_approval=mock.AsyncMock(return_value=False),
            get_archive_skip_logged=mock.Mock(return_value=set()),
            send_typing_pulse=send_typing_pulse,
        )
        runtime_deps = cast(
            bot_session_mirror_runtime.SessionMirrorRuntimeDeps[FakeChannel],
            runtime_deps_mock,
        )
        runtime = bot_session_mirror_runtime.SessionMirrorRuntime(runtime_deps)
        owner = mock.Mock()

        async def exercise_runtime_callback(target: object, *, deps: object) -> None:
            _ = target
            callback = cast(
                session_mirror_target.SessionMirrorTargetDeps[object, object, object, FakeChannel],
                deps,
            ).send_typing_pulse
            await callback(FakeChannel(), "thread-1", "session_mirror_busy")

        with mock.patch.object(
            session_mirror_target,
            "mirror_session_target",
            exercise_runtime_callback,
        ):
            await runtime.mirror_session_target(
                cast(bot_session_mirror_runtime.SessionMirrorOwner[FakeChannel], owner),
                {"codex_thread_id": "thread-1", "discord_thread_id": 222},
            )

        self.assertEqual(typing_pulses, [(222, "thread-1", "session_mirror_busy")])

    async def test_active_output_target_without_events_sends_typing_pulse(self) -> None:
        channels: list[int] = []
        typing_pulses: list[tuple[int, str, str]] = []
        logs: list[str] = []

        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            _ = session_path.write_text("", encoding="utf-8")

            async def resolve_channel(discord_thread_id: int) -> FakeChannel | None:
                channels.append(discord_thread_id)
                return FakeChannel()

            async def send_typing_pulse(channel: FakeChannel, target_thread_id: str, context: str) -> None:
                typing_pulses.append((channel.id, target_thread_id, context))

            def collect_items(
                codex_thread_id: str,
                events: list[JsonEvent],
                *,
                seen_agent_messages: dict[str, float],
                seen_user_messages: dict[str, float],
            ) -> list[item_sender.SessionMirrorItem]:
                _ = codex_thread_id, events, seen_agent_messages, seen_user_messages
                return []

            async def send_item(
                channel: FakeChannel,
                item: Mapping[str, str],
                *,
                target_thread_id: str,
                target_ref: str,
            ) -> None:
                _ = channel, item, target_thread_id, target_ref

            await session_mirror_target.mirror_session_target(
                {"codex_thread_id": "thread-1", "discord_thread_id": 222},
                deps=session_mirror_target.SessionMirrorTargetDeps(
                    parse_session_mirror_target=session_mirror.parse_session_mirror_target,
                    choose_thread=lambda thread_id, cwd: FakeThread(str(session_path)),
                    get_thread_context_usage=lambda thread: object(),
                    should_recommend_archive=lambda thread, usage: False,
                    get_thread_rollout_path=lambda thread: thread.rollout_path,
                    is_active_output_target=lambda thread_id: thread_id == "thread-1",
                    archive_skip_logged=set(),
                    is_pending_cursor_target=lambda thread_id: False,
                    clear_pending_cursor_target=lambda thread_id: None,
                    update_session_mirror_cursor=lambda thread_id, rollout_path, cursor: None,
                    get_or_init_session_mirror_cursor=lambda thread_id, rollout_path, initial_cursor: 0,
                    read_new_session_events=lambda session_path, cursor, max_events=None: ([], 0),
                    get_archive_backlog_max_events=lambda: 10,
                    collect_session_mirror_items=collect_items,
                    get_seen_agent_messages=lambda thread_id: {},
                    get_seen_user_messages=lambda thread_id: {},
                    resolve_session_mirror_channel=resolve_channel,
                    resolve_target_ref=lambda thread_id: (thread_id, thread_id),
                    has_session_mirror_event=lambda digest, thread_id: False,
                    send_session_mirror_item=send_item,
                    claim_session_mirror_event=lambda digest, thread_id: True,
                    release_session_mirror_output_target=release_output_target,
                    send_typing_pulse=send_typing_pulse,
                    log=logs.append,
                ),
            )

        self.assertEqual(channels, [222])
        self.assertEqual(typing_pulses, [(222, "thread-1", "session_mirror_busy")])
        self.assertEqual(logs, ["session_mirror_typing_pulse target=thread-1 channel=222"])


if __name__ == "__main__":
    _ = unittest.main()
