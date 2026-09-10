from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from dataclasses import dataclass

import codex_discord_stream as discord_stream
import codex_discord_stream_relay as discord_stream_relay


@dataclass(frozen=True, slots=True)
class DiscordAskRelayClassDeps:
    commentary_enabled: Callable[[], bool]
    send_chunks: discord_stream_relay.SendChunksFunc
    parse_interactive_notice: discord_stream_relay.ParseInteractiveNoticeFunc
    send_interactive_prompt: discord_stream_relay.SendInteractivePromptFunc
    register_discord_relay: discord_stream_relay.RegisterDiscordRelayFunc
    is_discord_relay_stale: discord_stream_relay.IsDiscordRelayStaleFunc
    had_steering_handoff_since: discord_stream_relay.HadSteeringHandoffSinceFunc
    log: discord_stream_relay.LogFunc
    format_log_text_len: discord_stream_relay.FormatLogTextLenFunc


def make_discord_ask_relay_class(
    deps: DiscordAskRelayClassDeps,
    *,
    quiet_notice_delay_seconds: float,
) -> type[discord_stream.DiscordAskRelay]:
    class DiscordAskRelay(discord_stream.DiscordAskRelay):
        def __init__(
            self,
            loop: asyncio.AbstractEventLoop,
            channel: discord_stream_relay.RelayChannel,
            target_thread_id: str | None,
            target_ref: str,
            quiet_notice_delay_sec: float = quiet_notice_delay_seconds,
            suppress_after_steering_since: float | None = None,
            send_timeout_blocks: bool = True,
            send_commentary_blocks: bool | None = None,
            send_final_blocks: bool = True,
        ) -> None:
            if send_commentary_blocks is None:
                send_commentary_blocks = deps.commentary_enabled()
            super().__init__(
                loop,
                channel,
                target_thread_id,
                target_ref,
                quiet_notice_delay_sec=quiet_notice_delay_sec,
                suppress_after_steering_since=suppress_after_steering_since,
                send_timeout_blocks=send_timeout_blocks,
                send_commentary_blocks=send_commentary_blocks,
                send_final_blocks=send_final_blocks,
                send_chunks_func=deps.send_chunks,
                parse_interactive_notice_func=deps.parse_interactive_notice,
                send_interactive_prompt_func=deps.send_interactive_prompt,
                register_discord_relay_func=deps.register_discord_relay,
                is_discord_relay_stale_func=deps.is_discord_relay_stale,
                had_steering_handoff_since_func=deps.had_steering_handoff_since,
                log_func=deps.log,
                format_log_text_len_func=deps.format_log_text_len,
            )

    return DiscordAskRelay
