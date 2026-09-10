from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeAlias, TypeVar

import codex_discord_prompt_busy_result as busy_result
import codex_discord_prompt_mapped_delivery as mapped_delivery
from codex_thread_models import ThreadInfo

ChannelT = TypeVar("ChannelT")
ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
RecentOffsets: TypeAlias = busy_result.RecentOffsets
TargetResolver: TypeAlias = Callable[[str | None], tuple[str | None, str]]
DeliverySnapshotter: TypeAlias = Callable[[str | None], tuple[ThreadInfo | None, RecentOffsets]]
MappedDeliveryDepsFactory: TypeAlias = Callable[[], mapped_delivery.MappedPromptDeliveryDeps[ChannelT]]


class AskStartMessageBuilder(Protocol):
    def __call__(self, prompt: str, *, queued: bool = False) -> str: ...


class ChunkSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        context: str | None = None,
    ) -> Awaitable[None]: ...


class ContextExhaustionNotifier(Protocol[ChannelContraT]):
    def __call__(self, channel: ChannelContraT, target_thread_id: str | None, target_ref: str) -> Awaitable[bool]: ...


class SessionMirrorDelegationPreparer(Protocol[ChannelContraT]):
    def __call__(self, channel: ChannelContraT, target_thread_id: str | None) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class PromptDeliveryPreparationRequest:
    prompt: str
    queued: bool
    ack_sent: bool
    target_thread_id: str | None


@dataclass(frozen=True, slots=True)
class PromptDeliveryPreparationDeps(Generic[ChannelT]):
    send_chunks: ChunkSender[ChannelT]
    build_ask_start_message: AskStartMessageBuilder
    resolve_target_ref: TargetResolver
    send_context_exhausted_prompt_notice_if_needed: ContextExhaustionNotifier[ChannelT]
    make_mapped_prompt_delivery_deps: MappedDeliveryDepsFactory[ChannelT]
    prepare_session_mirror_delegation: SessionMirrorDelegationPreparer[ChannelT]
    snapshot_ask_prompt_delivery_state: DeliverySnapshotter


@dataclass(frozen=True, slots=True)
class PromptDeliveryPreparationResult:
    handled: bool
    target_thread_id: str | None
    target_ref: str
    recent_offsets: RecentOffsets
    delegate_to_session_mirror: bool
    mapped_result: mapped_delivery.MappedPromptDeliveryResult | None = None


async def prepare_prompt_delivery(
    channel: ChannelT,
    request: PromptDeliveryPreparationRequest,
    *,
    deps: PromptDeliveryPreparationDeps[ChannelT],
) -> PromptDeliveryPreparationResult:
    if not request.ack_sent:
        await deps.send_chunks(
            channel,
            deps.build_ask_start_message(request.prompt, queued=request.queued),
            context="ask_start",
        )
    target_thread_id, target_ref = deps.resolve_target_ref(request.target_thread_id)
    if await deps.send_context_exhausted_prompt_notice_if_needed(channel, target_thread_id, target_ref):
        return _handled(target_thread_id, target_ref)
    mapped_result = await mapped_delivery.handle_mapped_prompt_delivery(
        channel,
        request.prompt,
        target_thread_id,
        deps=deps.make_mapped_prompt_delivery_deps(),
    )
    if mapped_result.handled:
        return _handled(target_thread_id, target_ref, mapped_result=mapped_result)
    delegate_to_session_mirror = await deps.prepare_session_mirror_delegation(channel, target_thread_id)
    _target_thread, recent_offsets = await asyncio.to_thread(
        deps.snapshot_ask_prompt_delivery_state,
        target_thread_id,
    )
    return PromptDeliveryPreparationResult(
        handled=False,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        recent_offsets=recent_offsets,
        delegate_to_session_mirror=delegate_to_session_mirror,
        mapped_result=None,
    )


def _handled(
    target_thread_id: str | None,
    target_ref: str,
    *,
    mapped_result: mapped_delivery.MappedPromptDeliveryResult | None = None,
) -> PromptDeliveryPreparationResult:
    recent_offsets: RecentOffsets = {}
    return PromptDeliveryPreparationResult(
        handled=True,
        target_thread_id=target_thread_id,
        target_ref=target_ref,
        recent_offsets=recent_offsets,
        delegate_to_session_mirror=False,
        mapped_result=mapped_result,
    )


def discarded_prompt_delivery(
    target_thread_id: str | None,
    target_ref: str,
) -> PromptDeliveryPreparationResult:
    return _handled(target_thread_id, target_ref)
