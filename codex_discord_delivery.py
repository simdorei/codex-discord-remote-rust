from __future__ import annotations

import io

import discord
from typing import Protocol, TypeVar

from codex_discord_delivery_state import (
    DISCORD_CHUNK_MARKERS_ENABLED,
    DISCORD_RESTARTING_ERROR,
    DISCORD_RESTARTING_NOTICE,
    DISCORD_SEND_RETRY_DELAYS_SECONDS,
    DiscordDeliveryRejected,
    DiscordDeliveryState,
    DiscordIdValue,
    LogFunc,
    Messageable,
    begin_cross_path_delivery,
    begin_discord_delivery,
    build_delivery_id,
    clear_discord_delivery_stopping,
    end_discord_delivery,
    finish_cross_path_delivery,
    get_interaction_command_name,
    get_messageable_id,
    is_discord_delivery_stopping,
    set_discord_delivery_stopping,
    sleep_discord_delivery_retry,
    split_delivery_chunks,
    wait_for_discord_delivery_drain,
)
from codex_discord_delivery_interactions import (
    adapt_discord_interaction,
    send_direct_followup,
    send_followup_chunks,
    send_interaction_not_allowed,
    send_interaction_response_tracked,
)
from codex_discord_text import format_discord_command_label, format_log_text_len

SentMessageT_co = TypeVar("SentMessageT_co", covariant=True)
SentMessageT = TypeVar("SentMessageT")


class DiscordMessageView(Protocol):
    pass


class TrackedMessageTarget(Protocol[SentMessageT_co]):
    @property
    def id(self) -> DiscordIdValue: ...

    async def send(self, content: str, **kwargs: DiscordMessageView | None) -> SentMessageT_co: ...


class AttachmentTarget(Protocol[SentMessageT_co]):
    @property
    def id(self) -> DiscordIdValue: ...

    async def send(self, content: str, *, file: discord.File) -> SentMessageT_co: ...


__all__ = [
    "DISCORD_CHUNK_MARKERS_ENABLED",
    "DISCORD_RESTARTING_ERROR",
    "DISCORD_RESTARTING_NOTICE",
    "DISCORD_SEND_RETRY_DELAYS_SECONDS",
    "DiscordDeliveryRejected",
    "DiscordDeliveryState",
    "adapt_discord_interaction",
    "begin_cross_path_delivery",
    "begin_discord_delivery",
    "build_delivery_id",
    "clear_discord_delivery_stopping",
    "end_discord_delivery",
    "finish_cross_path_delivery",
    "get_interaction_command_name",
    "get_messageable_id",
    "is_discord_delivery_stopping",
    "send_chunks",
    "send_attachment_bytes",
    "send_direct_followup",
    "send_discord_restarting_notice",
    "send_followup_chunks",
    "send_interaction_not_allowed",
    "send_interaction_response_tracked",
    "send_message_tracked",
    "set_discord_delivery_stopping",
    "split_delivery_chunks",
    "wait_for_discord_delivery_drain",
]


async def send_chunks(
    state: DiscordDeliveryState,
    target: Messageable,
    text: str,
    *,
    log_func: LogFunc,
    context: str = "send_chunks",
    allow_during_stop: bool = False,
) -> int:
    chunks = split_delivery_chunks(text, state=state)
    delivery_id = build_delivery_id(text)
    target_id = get_messageable_id(target)
    safe_context = format_discord_command_label(context, limit=120)
    delivery_token = begin_discord_delivery(
        state,
        f"chunks:{delivery_id}:{target_id}:{safe_context}",
        log_func=log_func,
        allow_during_stop=allow_during_stop,
    )
    cross_path_claim = None
    delivery_succeeded = False
    try:
        cross_path_start = await begin_cross_path_delivery(
            state,
            target_id=target_id,
            text=text,
            context=context,
        )
        cross_path_claim = cross_path_start.claim
        if cross_path_start.duplicate_source is not None:
            log_func(
                f"discord_delivery_duplicate_suppressed id={delivery_id} "
                + f"context={safe_context} target={target_id} "
                + f"previous={cross_path_start.duplicate_source}"
            )
            return 0
        log_func(
            f"discord_delivery_start id={delivery_id} context={safe_context} "
            + f"target={target_id} chunks={len(chunks)} text_len={format_log_text_len(text)}"
        )
        for index, chunk in enumerate(chunks, start=1):
            sent_message = None
            attempts = len(state.retry_delays_seconds) + 1
            for attempt in range(1, attempts + 1):
                try:
                    sent_message = await target.send(chunk)
                    break
                except (discord.DiscordException, OSError, RuntimeError) as exc:
                    if attempt >= attempts:
                        log_func(
                            f"discord_delivery_failed id={delivery_id} context={safe_context} "
                            + f"target={target_id} part={index}/{len(chunks)} attempt={attempt} "
                            + f"chunk_len={format_log_text_len(chunk)} error_type={type(exc).__name__}"
                        )
                        raise
                    log_func(
                        f"discord_delivery_retry id={delivery_id} context={safe_context} "
                        + f"target={target_id} part={index}/{len(chunks)} attempt={attempt} "
                        + f"chunk_len={format_log_text_len(chunk)} error_type={type(exc).__name__}"
                    )
                    await sleep_discord_delivery_retry(state.retry_delays_seconds[attempt - 1])
            log_func(
                f"discord_delivery_chunk_sent id={delivery_id} context={safe_context} "
                + f"target={target_id} part={index}/{len(chunks)} "
                + f"message={getattr(sent_message, 'id', '-') or '-'} "
                + f"chunk_len={format_log_text_len(chunk)}"
            )
        log_func(
            f"discord_delivery_sent id={delivery_id} context={safe_context} "
            + f"target={target_id} chunks={len(chunks)}"
        )
        delivery_succeeded = True
        return len(chunks)
    finally:
        finish_cross_path_delivery(
            state,
            claim=cross_path_claim,
            succeeded=delivery_succeeded,
        )
        end_discord_delivery(state, delivery_token)


async def send_discord_restarting_notice(
    state: DiscordDeliveryState,
    target: Messageable,
    *,
    log_func: LogFunc,
) -> None:
    _ = await send_chunks(
        state,
        target,
        DISCORD_RESTARTING_NOTICE,
        log_func=log_func,
        context="restart_notice",
        allow_during_stop=True,
    )


async def send_attachment_bytes(
    state: DiscordDeliveryState,
    target: AttachmentTarget[SentMessageT],
    content: str,
    filename: str,
    attachment_bytes: bytes,
    *,
    log_func: LogFunc,
    context: str = "send_attachment",
    allow_during_stop: bool = False,
) -> SentMessageT:
    target_id = get_messageable_id(target)
    safe_context = format_discord_command_label(context, limit=120)
    safe_filename = format_discord_command_label(filename, limit=120)
    delivery_id = build_delivery_id(f"{safe_context}:{safe_filename}:{len(attachment_bytes)}:{content}")
    delivery_token = begin_discord_delivery(
        state,
        f"attachment:{delivery_id}:{target_id}:{safe_context}",
        log_func=log_func,
        allow_during_stop=allow_during_stop,
    )
    try:
        log_func(
            f"discord_attachment_start id={delivery_id} context={safe_context} "
            + f"target={target_id} filename={safe_filename} bytes={len(attachment_bytes)} "
            + f"text_len={format_log_text_len(content)}"
        )
        attempts = len(state.retry_delays_seconds) + 1
        for attempt in range(1, attempts + 1):
            try:
                file = discord.File(io.BytesIO(attachment_bytes), filename=filename)
                sent_message = await target.send(content, file=file)
                log_func(
                    f"discord_attachment_sent id={delivery_id} context={safe_context} "
                    + f"target={target_id} message={getattr(sent_message, 'id', '-') or '-'} "
                    + f"filename={safe_filename} bytes={len(attachment_bytes)}"
                )
                return sent_message
            except (discord.DiscordException, OSError, RuntimeError) as exc:
                if attempt >= attempts:
                    log_func(
                        f"discord_attachment_failed id={delivery_id} context={safe_context} "
                        + f"target={target_id} attempt={attempt} filename={safe_filename} "
                        + f"bytes={len(attachment_bytes)} error_type={type(exc).__name__}"
                    )
                    raise
                log_func(
                    f"discord_attachment_retry id={delivery_id} context={safe_context} "
                    + f"target={target_id} attempt={attempt} filename={safe_filename} "
                    + f"bytes={len(attachment_bytes)} error_type={type(exc).__name__}"
                )
                await sleep_discord_delivery_retry(state.retry_delays_seconds[attempt - 1])
    finally:
        end_discord_delivery(state, delivery_token)
    raise RuntimeError("unreachable send_attachment_bytes retry loop")


async def send_message_tracked(
    state: DiscordDeliveryState,
    target: TrackedMessageTarget[SentMessageT],
    content: str,
    *,
    log_func: LogFunc,
    view: DiscordMessageView | None = None,
    context: str = "send_message",
    allow_during_stop: bool = False,
) -> SentMessageT:
    target_id = get_messageable_id(target)
    safe_context = format_discord_command_label(context, limit=120)
    delivery_id = build_delivery_id(f"{safe_context}:{content}")
    has_view = view is not None
    delivery_token = begin_discord_delivery(
        state,
        f"message:{delivery_id}:{target_id}:{safe_context}",
        log_func=log_func,
        allow_during_stop=allow_during_stop,
    )
    try:
        log_func(
            f"discord_message_send_start id={delivery_id} context={safe_context or '-'} "
            + f"target={target_id} has_view={has_view} text_len={format_log_text_len(content)}"
        )
        if has_view:
            sent_message = await target.send(content, view=view)
        else:
            sent_message = await target.send(content)
        log_func(
            f"discord_message_send_sent id={delivery_id} context={safe_context or '-'} "
            + f"target={target_id} message={getattr(sent_message, 'id', '-') or '-'} "
            + f"has_view={has_view}"
        )
        return sent_message
    except (discord.DiscordException, OSError, RuntimeError) as exc:
        log_func(
            f"discord_message_send_failed id={delivery_id} context={safe_context or '-'} "
            + f"target={target_id} has_view={has_view} text_len={format_log_text_len(content)} "
            + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
        )
        raise
    finally:
        end_discord_delivery(state, delivery_token)
