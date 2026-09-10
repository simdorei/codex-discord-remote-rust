from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Generic, TypeAlias, TypeVar

import codex_discord_session_mirror_commit as session_mirror_commit
import codex_discord_session_mirror_item_sender as session_mirror_item_sender

ChannelT = TypeVar("ChannelT")
SessionMirrorItem: TypeAlias = session_mirror_item_sender.SessionMirrorItem


@dataclass(frozen=True, slots=True)
class SessionMirrorDeliveryFlowDeps(Generic[ChannelT]):
    resolve_session_mirror_channel: Callable[[int], Awaitable[ChannelT | None]]
    resolve_target_ref: Callable[[str], tuple[str | None, str]]
    has_session_mirror_event: session_mirror_item_sender.SessionMirrorEventChecker
    send_session_mirror_item: session_mirror_item_sender.SessionMirrorItemSender[ChannelT]
    claim_session_mirror_event: session_mirror_item_sender.SessionMirrorEventClaimer
    update_session_mirror_cursor: session_mirror_commit.CursorUpdater
    release_session_mirror_output_target: session_mirror_commit.OutputTargetReleaser
    log: session_mirror_commit.LogFunc


async def deliver_and_commit_session_mirror_items(
    codex_thread_id: str,
    rollout_path: str,
    next_cursor: int,
    *,
    discord_thread_id: int,
    event_count: int,
    items: Sequence[SessionMirrorItem],
    deps: SessionMirrorDeliveryFlowDeps[ChannelT],
) -> bool:
    channel = await deps.resolve_session_mirror_channel(discord_thread_id)
    if channel is None:
        return False

    _resolved_thread_id, target_ref = deps.resolve_target_ref(codex_thread_id)
    send_result = await session_mirror_item_sender.send_unclaimed_session_mirror_items(
        channel,
        items,
        codex_thread_id=codex_thread_id,
        target_ref=target_ref,
        deps=session_mirror_item_sender.SessionMirrorItemSenderDeps(
            has_session_mirror_event=deps.has_session_mirror_event,
            send_session_mirror_item=deps.send_session_mirror_item,
            claim_session_mirror_event=deps.claim_session_mirror_event,
        ),
    )
    await session_mirror_commit.commit_session_mirror_delivery(
        codex_thread_id,
        rollout_path,
        next_cursor,
        discord_thread_id=discord_thread_id,
        event_count=event_count,
        sent_count=send_result.sent_count,
        terminal_sent=send_result.terminal_sent,
        deps=session_mirror_commit.SessionMirrorCommitDeps(
            update_session_mirror_cursor=deps.update_session_mirror_cursor,
            release_session_mirror_output_target=deps.release_session_mirror_output_target,
            log=deps.log,
        ),
    )
    return True
