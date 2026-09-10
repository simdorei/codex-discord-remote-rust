from __future__ import annotations

import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import codex_discord_steering as discord_steering
import codex_discord_steering_watch as discord_steering_watch
from codex_thread_models import ThreadInfo


class ApprovalWatchBridge(Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...

    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str: ...


ExceptionTypes = tuple[type[Exception], ...]


@dataclass(frozen=True, slots=True)
class SteeringWatchRuntimeDeps:
    make_relay: discord_steering_watch.MakeRelayFunc
    get_watch_timeout: Callable[[], float]
    channel_typing: discord_steering_watch.ChannelTypingFunc
    run_watch_stream: discord_steering_watch.WatchStreamFunc
    send_chunks: discord_steering_watch.SendChunksFunc
    log_line: Callable[[str], None]
    format_log_text_len: Callable[[str], discord_steering_watch.LogLengthValue]


def make_steering_watch_runtime_deps(
    *,
    make_relay: discord_steering_watch.MakeRelayFunc,
    get_watch_timeout: Callable[[], float],
    channel_typing: discord_steering_watch.ChannelTypingFunc,
    run_watch_stream: discord_steering_watch.WatchStreamFunc,
    send_chunks: discord_steering_watch.SendChunksFunc,
    log_line: Callable[[str], None],
    format_log_text_len: Callable[[str], discord_steering_watch.LogLengthValue],
) -> SteeringWatchRuntimeDeps:
    return SteeringWatchRuntimeDeps(
        make_relay=make_relay,
        get_watch_timeout=get_watch_timeout,
        channel_typing=channel_typing,
        run_watch_stream=run_watch_stream,
        send_chunks=send_chunks,
        log_line=log_line,
        format_log_text_len=format_log_text_len,
    )


async def stream_steering_prompt_result_to_channel(
    channel: discord_steering_watch.SteeringWatchChannel,
    steering_result: discord_steering.SteeringPromptResult | None,
    target_thread_id: str | None,
    *,
    label: str = "Steering",
    send_commentary_blocks: bool | None = None,
    send_final_blocks: bool = True,
    deps: SteeringWatchRuntimeDeps,
) -> bool:
    watch_result = steering_result if isinstance(steering_result, discord_steering.SteeringPromptResult) else None
    return await discord_steering_watch.stream_steering_prompt_result_to_channel(
        channel,
        watch_result,
        target_thread_id,
        label=label,
        send_commentary_blocks=send_commentary_blocks,
        send_final_blocks=send_final_blocks,
        deps=discord_steering_watch.SteeringWatchDeps(
            monotonic=time.monotonic,
            make_relay=deps.make_relay,
            get_watch_timeout=deps.get_watch_timeout,
            channel_typing=deps.channel_typing,
            run_watch_stream=deps.run_watch_stream,
            send_chunks=deps.send_chunks,
            log_line=deps.log_line,
            format_log_text_len=deps.format_log_text_len,
        ),
    )


def make_post_approval_watch_result(
    target_thread_id: str,
    *,
    bridge: ApprovalWatchBridge,
    get_active_turn_id: Callable[[str], str | None],
    log_line: Callable[[str], None],
    expected_exceptions: ExceptionTypes,
) -> discord_steering.SteeringPromptResult | None:
    try:
        thread = bridge.choose_thread(target_thread_id, None)
        session_path = Path(thread.rollout_path)
        if not session_path.exists():
            log_line(
                f"approval_followup_watch_unavailable target={target_thread_id} "
                + f"reason=session_missing path={session_path}"
            )
            return None
        turn_id = get_active_turn_id(thread.id)
        if not turn_id:
            log_line(
                f"approval_followup_watch_unavailable target={target_thread_id} "
                + "reason=exact_active_turn_unavailable"
            )
            return None
        return discord_steering.SteeringPromptResult(
            0,
            "[approval_submitted]",
            target_thread_id=thread.id,
            target_ref=bridge.get_thread_workspace_ref(thread),
            session_path=str(session_path),
            start_offset=session_path.stat().st_size,
            watch_target=discord_steering.NativeExactWatchTarget(turn_id),
        )
    except expected_exceptions as exc:
        log_line(
            f"approval_followup_watch_unavailable target={target_thread_id} "
            + f"error_type={type(exc).__name__}"
        )
        return None
