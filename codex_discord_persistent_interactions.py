from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_components import parse_approval_custom_id, parse_input_choice_custom_id
from codex_discord_steering import SteeringPromptResult

Submitter = Callable[[str, str], tuple[int, str]]
LogFunc = Callable[[str], None]
TextLenFormatter = Callable[[object], str]
ApprovalWatchResult = SteeringPromptResult | None


class PersistentInteractionUser(Protocol):
    @property
    def id(self) -> int: ...


class PersistentInteractionResponse(Protocol):
    def defer(self, thinking: bool = False, **kwargs: bool) -> Awaitable[None]: ...


class PersistentInteraction(Protocol):
    @property
    def user(self) -> PersistentInteractionUser: ...

    @property
    def response(self) -> PersistentInteractionResponse: ...


class ComponentClearer(Protocol):
    def __call__(self, interaction: PersistentInteraction, *, context: str) -> Awaitable[None]: ...


class InteractionResponseSender(Protocol):
    def __call__(
        self,
        interaction: PersistentInteraction,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> Awaitable[None]: ...


class FollowupChunksSender(Protocol):
    def __call__(
        self,
        interaction: PersistentInteraction,
        text: str,
        *,
        title: str,
        exit_code: int,
        log_prefix: str,
    ) -> Awaitable[None]: ...


class ApprovalResultStreamer(Protocol):
    def __call__(
        self,
        interaction: PersistentInteraction,
        watch_result: ApprovalWatchResult,
        target_thread_id: str,
    ) -> Awaitable[bool]: ...


@dataclass(frozen=True, slots=True)
class PersistentApprovalDeps:
    is_user_allowed: Callable[[int], bool]
    claim_component: Callable[[PersistentInteraction, str], bool]
    clear_components: ComponentClearer
    send_response: InteractionResponseSender
    send_followup_chunks: FollowupChunksSender
    make_watch_result: Callable[[str], ApprovalWatchResult]
    stream_post_approval_result: ApprovalResultStreamer
    format_log_text_len: TextLenFormatter
    log: LogFunc


@dataclass(frozen=True, slots=True)
class PersistentInputChoiceDeps:
    is_user_allowed: Callable[[int], bool]
    claim_component: Callable[[PersistentInteraction, str], bool]
    clear_components: ComponentClearer
    send_response: InteractionResponseSender
    send_followup_chunks: FollowupChunksSender
    format_log_text_len: TextLenFormatter
    log: LogFunc


async def handle_persistent_approval_interaction(
    interaction: PersistentInteraction,
    custom_id: str,
    *,
    approval_submitter: Submitter,
    deps: PersistentApprovalDeps,
) -> bool:
    parsed = parse_approval_custom_id(custom_id)
    if not parsed:
        return False
    target_thread_id, answer = parsed
    user_id = int(interaction.user.id or 0)
    if not deps.is_user_allowed(user_id):
        await deps.send_response(
            interaction,
            "This user is not allowed.",
            ephemeral=True,
            context="approval_persistent_denied",
        )
        deps.log(f"approval_persistent_denied user={user_id} target={target_thread_id}")
        return True
    if not deps.claim_component(interaction, custom_id):
        await deps.clear_components(interaction, context="approval_persistent_already_handled")
        await deps.send_response(
            interaction,
            "This approval choice was already handled.",
            ephemeral=True,
            context="approval_persistent_already_handled",
        )
        deps.log(f"approval_persistent_already_handled user={user_id} target={target_thread_id}")
        return True
    await interaction.response.defer(thinking=True)
    await deps.clear_components(interaction, context="approval_persistent")
    answer_len = deps.format_log_text_len(answer)
    deps.log(f"approval_persistent user={user_id} target={target_thread_id} answer_len={answer_len}")
    watch_result = deps.make_watch_result(target_thread_id)
    exit_code, output = await asyncio.to_thread(approval_submitter, target_thread_id, answer)
    deps.log(f"approval_persistent_done exit={exit_code} target={target_thread_id} answer_len={answer_len}")
    title = "Approval submitted" if exit_code == 0 else f"Approval failed (exit {exit_code})"
    await deps.send_followup_chunks(
        interaction,
        f"{title}\n\n{output or '(no output)'}",
        title="Approval",
        exit_code=exit_code,
        log_prefix="button_response",
    )
    if exit_code == 0:
        _ = await deps.stream_post_approval_result(interaction, watch_result, target_thread_id)
    return True


async def handle_persistent_input_choice_interaction(
    interaction: PersistentInteraction,
    custom_id: str,
    *,
    input_submitter: Submitter,
    deps: PersistentInputChoiceDeps,
) -> bool:
    parsed = parse_input_choice_custom_id(custom_id)
    if not parsed:
        return False
    target_thread_id, value = parsed
    user_id = int(interaction.user.id or 0)
    if not deps.is_user_allowed(user_id):
        await deps.send_response(
            interaction,
            "This user is not allowed.",
            ephemeral=True,
            context="input_choice_persistent_denied",
        )
        deps.log(f"input_choice_persistent_denied user={user_id} target={target_thread_id}")
        return True
    if not deps.claim_component(interaction, custom_id):
        await deps.clear_components(interaction, context="input_choice_persistent_already_handled")
        await deps.send_response(
            interaction,
            "This input choice was already handled.",
            ephemeral=True,
            context="input_choice_persistent_already_handled",
        )
        deps.log(f"input_choice_persistent_already_handled user={user_id} target={target_thread_id}")
        return True
    await interaction.response.defer(thinking=True)
    await deps.clear_components(interaction, context="input_choice_persistent")
    value_len = deps.format_log_text_len(value)
    deps.log(f"input_choice_persistent user={user_id} target={target_thread_id} value_len={value_len}")
    exit_code, output = await asyncio.to_thread(input_submitter, target_thread_id, value)
    deps.log(f"input_choice_persistent_done exit={exit_code} target={target_thread_id} value_len={value_len}")
    title = "Input submitted" if exit_code == 0 else f"Input failed (exit {exit_code})"
    await deps.send_followup_chunks(
        interaction,
        f"{title}\n\n{output or '(no output)'}",
        title="Input",
        exit_code=exit_code,
        log_prefix="button_response",
    )
    return True
