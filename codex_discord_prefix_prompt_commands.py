from __future__ import annotations

from collections.abc import Callable
from typing import Final

from codex_discord_prefix_skill_prompts import (
    build_archive_used_prompt,
    DEEP_INTERVIEW_PROMPT_HEADER,
    build_deep_interview_prompt,
)
from codex_discord_prefix_prompt_types import (
    AuthorLike,
    ChannelLike,
    HandlePlainAskFunc,
    MessageLike,
    PrefixPromptCommandDeps,
    SendChunksFunc,
)

__all__ = [
    "DEEP_INTERVIEW_PROMPT_HEADER",
    "ARCHIVE_USED_COMMANDS",
    "AuthorLike",
    "ChannelLike",
    "HandlePlainAskFunc",
    "INTERVIEW_COMMANDS",
    "MessageLike",
    "PrefixPromptCommandDeps",
    "PRO_COMMANDS",
    "SendChunksFunc",
    "build_archive_used_prompt",
    "build_deep_interview_prompt",
    "handle_prefix_prompt_command",
]

INTERVIEW_COMMANDS = {"interview", "deep_interview", "deep-interview"}
ARCHIVE_USED_COMMANDS = {"archive-used"}
PRO_COMMANDS: Final = frozenset({"pro"})


async def handle_prefix_prompt_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixPromptCommandDeps,
) -> bool:
    if command in PRO_COMMANDS:
        return await _handle_skill_prompt_command(
            command,
            arg,
            message,
            deps=deps,
            usage_context="prefix_pro_usage",
            log_context="prefix_pro",
            requires_request=True,
            build_prompt=lambda request: f"!pro {request}",
        )
    if command in INTERVIEW_COMMANDS:
        return await _handle_skill_prompt_command(
            command,
            arg,
            message,
            deps=deps,
            usage_context="prefix_interview_usage",
            log_context="prefix_interview",
            requires_request=True,
            build_prompt=build_deep_interview_prompt,
        )
    if command in ARCHIVE_USED_COMMANDS:
        return await _handle_skill_prompt_command(
            command,
            arg,
            message,
            deps=deps,
            usage_context="prefix_archive_used_usage",
            log_context="prefix_archive_used",
            requires_request=True,
            build_prompt=build_archive_used_prompt,
        )
    return False


async def _handle_skill_prompt_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixPromptCommandDeps,
    usage_context: str,
    log_context: str,
    requires_request: bool,
    build_prompt: Callable[[str], str],
) -> bool:
    if requires_request and not arg:
        usage_arg = "threshold" if command in ARCHIVE_USED_COMMANDS else "request"
        _ = await deps.send_chunks(
            message.channel,
            f"Usage: !{deps.format_discord_command_label(command)} <{usage_arg}>",
            context=usage_context,
        )
        return True
    target_thread_id = await _resolve_target_or_send_project_message(message, deps=deps)
    if target_thread_id is None:
        return True
    log_message = (
        f"{log_context} channel={message.channel.id} user={message.author.id} "
        + f"target={target_thread_id or '-'} prompt_len={deps.format_log_text_len(arg)}"
    )
    deps.log_line(log_message)
    await deps.handle_plain_ask(message, build_prompt(arg), target_thread_id=target_thread_id)
    return True


async def _resolve_target_or_send_project_message(
    message: MessageLike,
    *,
    deps: PrefixPromptCommandDeps,
) -> str | None:
    target_thread_id = deps.get_mirrored_codex_thread_id(message.channel.id)
    if target_thread_id is not None:
        return target_thread_id
    project_message = deps.describe_mirrored_project_channel(message.channel.id)
    if project_message:
        _ = await deps.send_chunks(message.channel, project_message)
    return None
