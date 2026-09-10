from __future__ import annotations

from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
SessionMirrorItem = Mapping[str, str]
SessionMirrorEventChecker = Callable[[str, str], Awaitable[bool]]
SessionMirrorEventClaimer = Callable[[str, str], Awaitable[bool]]


class SessionMirrorItemSender(Protocol[ChannelT_contra]):
    def __call__(
        self,
        channel: ChannelT_contra,
        item: SessionMirrorItem,
        *,
        target_thread_id: str,
        target_ref: str,
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class SessionMirrorItemSendResult:
    sent_count: int
    terminal_sent: bool


@dataclass(frozen=True, slots=True)
class SessionMirrorItemSenderDeps(Generic[ChannelT]):
    has_session_mirror_event: SessionMirrorEventChecker
    send_session_mirror_item: SessionMirrorItemSender[ChannelT]
    claim_session_mirror_event: SessionMirrorEventClaimer


async def send_unclaimed_session_mirror_items(
    channel: ChannelT,
    items: Sequence[SessionMirrorItem],
    *,
    codex_thread_id: str,
    target_ref: str,
    deps: SessionMirrorItemSenderDeps[ChannelT],
) -> SessionMirrorItemSendResult:
    sent_count = 0
    terminal_sent = False
    delivery_target_ref = target_ref or codex_thread_id
    for item in items:
        digest = item.get("digest") or ""
        terminal_item = item.get("kind") == "final"
        if digest and await deps.has_session_mirror_event(digest, codex_thread_id):
            continue
        await deps.send_session_mirror_item(
            channel,
            item,
            target_thread_id=codex_thread_id,
            target_ref=delivery_target_ref,
        )
        if digest:
            _ = await deps.claim_session_mirror_event(digest, codex_thread_id)
        sent_count += 1
        if terminal_item:
            terminal_sent = True
    return SessionMirrorItemSendResult(sent_count=sent_count, terminal_sent=terminal_sent)
