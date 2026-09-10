from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from typing import TypeVar

import codex_discord_prompt_mapped_delivery as discord_prompt_mapped_delivery


ChannelT = TypeVar("ChannelT")
SyncTransportNoWait = Callable[[str, str | None], tuple[int, str]]


def make_mapped_prompt_delivery_deps(
    *,
    prepare_mapped_session_mirror_output: discord_prompt_mapped_delivery.PrepareMappedSessionMirrorOutput[ChannelT],
    set_selected_thread_id: discord_prompt_mapped_delivery.SelectedThreadSetter,
    channel_typing: discord_prompt_mapped_delivery.ChannelTyping[ChannelT],
    run_transport_prompt_no_wait: SyncTransportNoWait,
    send_chunks: discord_prompt_mapped_delivery.ChunkSender[ChannelT],
    is_delivery_confirmation_timeout: discord_prompt_mapped_delivery.OutputPredicate,
    format_pending_ask_delivery_output: discord_prompt_mapped_delivery.PendingFormatter,
    release_session_mirror_output_target: discord_prompt_mapped_delivery.OutputTargetReleaser,
    is_selected_thread_busy_error: discord_prompt_mapped_delivery.BusyPredicate,
    send_codex_app_menu_if_available: discord_prompt_mapped_delivery.AppMenuSender[ChannelT],
    send_resume_failure: discord_prompt_mapped_delivery.ResumeFailureSender[ChannelT],
    format_log_text_len: discord_prompt_mapped_delivery.TextLenFunc,
    log: discord_prompt_mapped_delivery.LogFunc,
    preprocess_prompt: discord_prompt_mapped_delivery.PromptPreprocessor = discord_prompt_mapped_delivery.keep_prompt,
    mark_recent_discord_origin_prompt: discord_prompt_mapped_delivery.DiscordOriginPromptMarker = (
        discord_prompt_mapped_delivery.ignore_discord_origin_prompt
    ),
) -> discord_prompt_mapped_delivery.MappedPromptDeliveryDeps[ChannelT]:
    async def run_transport_no_wait(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        return await asyncio.to_thread(
            run_transport_prompt_no_wait,
            prompt,
            target_thread_id,
        )

    return discord_prompt_mapped_delivery.MappedPromptDeliveryDeps(
        prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
        set_selected_thread_id=set_selected_thread_id,
        channel_typing=channel_typing,
        preprocess_prompt=preprocess_prompt,
        mark_recent_discord_origin_prompt=mark_recent_discord_origin_prompt,
        run_transport_prompt_no_wait=run_transport_no_wait,
        send_chunks=send_chunks,
        is_delivery_confirmation_timeout=is_delivery_confirmation_timeout,
        format_pending_ask_delivery_output=format_pending_ask_delivery_output,
        release_session_mirror_output_target=release_session_mirror_output_target,
        is_selected_thread_busy_error=is_selected_thread_busy_error,
        send_codex_app_menu_if_available=send_codex_app_menu_if_available,
        send_resume_failure=send_resume_failure,
        format_log_text_len=format_log_text_len,
        log=log,
    )
