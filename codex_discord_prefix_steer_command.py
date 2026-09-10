from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Protocol

from codex_discord_steering import SteeringPromptResult

STEER_COMMAND = "steer"


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class AuthorLike(Protocol):
    @property
    def id(self) -> int:
        ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike:
        ...

    @property
    def author(self) -> AuthorLike:
        ...


class SendChunksResult(Protocol):
    pass


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[SendChunksResult]:
        ...


class ChannelTypingFunc(Protocol):
    def __call__(
        self,
        channel: ChannelLike,
        *,
        context: str = "typing",
    ) -> AbstractAsyncContextManager[None]:
        ...


class RunSteeringPromptFunc(Protocol):
    def __call__(self, prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
        ...


class StreamSteeringPromptResultFunc(Protocol):
    def __call__(
        self,
        channel: ChannelLike,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> Awaitable[bool]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixSteerCommandDeps:
    send_chunks: SendChunksFunc
    qa_commands_enabled: Callable[[], bool]
    get_mirrored_codex_thread_id: Callable[[int], str | None]
    resolve_selected_target: Callable[[], tuple[str | None, str]]
    prepare_mapped_session_mirror_output: Callable[[ChannelLike, str | None], Awaitable[bool]]
    prepare_session_mirror_delegation: Callable[[ChannelLike, str | None], Awaitable[bool]]
    channel_typing: ChannelTypingFunc
    run_steering_prompt: RunSteeringPromptFunc
    mark_steering_handoff: Callable[[str | None], None]
    stream_steering_prompt_result_to_channel: StreamSteeringPromptResultFunc
    log_line: Callable[[str], None]
    format_log_text_len: Callable[[str], str]
    monotonic: Callable[[], float]


async def handle_prefix_steer_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixSteerCommandDeps,
) -> bool:
    if command != STEER_COMMAND:
        return False
    if not deps.qa_commands_enabled():
        _ = await deps.send_chunks(
            message.channel,
            "Discord QA steering is disabled. Set DISCORD_ENABLE_QA_COMMANDS=1 to enable it.",
            context="prefix_steer_disabled",
        )
        return True
    if not arg:
        _ = await deps.send_chunks(message.channel, "Usage: !steer <prompt>", context="prefix_steer_usage")
        return True

    target_thread_id = deps.get_mirrored_codex_thread_id(message.channel.id)
    if target_thread_id is None:
        target_thread_id, _target_ref = deps.resolve_selected_target()
    if not target_thread_id:
        _ = await deps.send_chunks(
            message.channel,
            "No Codex thread target found.",
            context="prefix_steer_no_target",
        )
        return True

    deps.log_line(
        f"prefix_steer channel={message.channel.id} user={message.author.id} "
        + f"target={target_thread_id} prompt_len={deps.format_log_text_len(arg)}"
    )
    delegate_to_session_mirror = await deps.prepare_mapped_session_mirror_output(
        message.channel,
        target_thread_id,
    )
    if not delegate_to_session_mirror:
        delegate_to_session_mirror = await deps.prepare_session_mirror_delegation(
            message.channel,
            target_thread_id,
        )

    started_at = deps.monotonic()
    async with deps.channel_typing(message.channel, context="prefix_steer"):
        steering_result: SteeringPromptResult = await asyncio.to_thread(
            deps.run_steering_prompt,
            arg,
            target_thread_id,
        )
    exit_code = steering_result.exit_code
    output = steering_result.output
    if exit_code == 0:
        deps.mark_steering_handoff(target_thread_id)
    deps.log_line(
        f"prefix_steer_done exit={exit_code} target={target_thread_id} "
        + f"elapsed_sec={deps.monotonic() - started_at:.2f} output_len={deps.format_log_text_len(output)}"
    )

    title = "Steering sent" if exit_code == 0 else f"Steering failed (exit {exit_code})"
    _ = await deps.send_chunks(message.channel, f"{title}\n\n{output or '(no output)'}")
    if exit_code == 0:
        if delegate_to_session_mirror:
            deps.log_line(f"prefix_steer_delegated_to_session_mirror target={target_thread_id}")
        _ = await deps.stream_steering_prompt_result_to_channel(
            message.channel,
            steering_result,
            target_thread_id,
            send_commentary_blocks=False if delegate_to_session_mirror else None,
            send_final_blocks=not delegate_to_session_mirror,
        )
    return True
