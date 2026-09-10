from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

APPROVAL_COMMANDS = {"approval", "approve"}


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]:
        ...


class SendInteractivePromptFunc(Protocol):
    def __call__(
        self,
        channel: ChannelLike,
        target_thread_id: str | None,
        target_ref: str,
        state: str,
        prompt_text: str,
        choices: list[str],
    ) -> Awaitable[None]:
        ...


@dataclass(frozen=True, slots=True)
class PrefixApprovalCommandDeps:
    send_chunks: SendChunksFunc
    get_mirrored_codex_thread_id: Callable[[int], str | None]
    resolve_selected_target: Callable[[], tuple[str | None, str]]
    get_interactive_state_for_thread: Callable[[str | None], tuple[str, str | None, str]]
    build_where_message: Callable[[int | None], str]
    send_interactive_prompt: SendInteractivePromptFunc
    interactive_state_approval: str


async def handle_prefix_approval_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixApprovalCommandDeps,
) -> bool:
    _ = arg
    if command not in APPROVAL_COMMANDS:
        return False

    target_thread_id = deps.get_mirrored_codex_thread_id(message.channel.id)
    if not target_thread_id:
        target_thread_id, _target_ref = deps.resolve_selected_target()
    if not target_thread_id:
        _ = await deps.send_chunks(message.channel, "No Codex thread target found.", context="prefix_approval_no_target")
        return True

    state, resolved_thread_id, target_ref = deps.get_interactive_state_for_thread(target_thread_id)
    if state != deps.interactive_state_approval or not resolved_thread_id:
        _ = await deps.send_chunks(
            message.channel,
            "\n".join(
                [
                    "No pending approval for this Codex thread.",
                    deps.build_where_message(message.channel.id),
                ]
            ),
        )
        return True

    _ = await deps.send_interactive_prompt(
        message.channel,
        resolved_thread_id,
        target_ref,
        deps.interactive_state_approval,
        "Pending approval",
        [],
    )
    return True
