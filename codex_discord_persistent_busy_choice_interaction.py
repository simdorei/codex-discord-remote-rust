from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol

import codex_discord_persistent_busy_choice as discord_persistent_busy_choice
import codex_discord_persistent_busy_queue as discord_persistent_busy_queue
import codex_discord_persistent_busy_steer_action as discord_persistent_busy_steer_action
import codex_discord_busy_choice_stop_action as discord_busy_choice_stop_action

LogFunc = Callable[[str], None]
BusyChoiceRecordGetter = Callable[[str], discord_persistent_busy_choice.BusyChoiceRecord | None]
BusyChoiceRecordClaimer = Callable[[str], bool]


class PersistentBusyUser(Protocol):
    @property
    def id(self) -> int | str | None: ...


class PersistentBusyResponse(Protocol):
    def defer(self, *, thinking: bool, ephemeral: bool) -> Awaitable[None]: ...


class PersistentBusyInteraction(discord_persistent_busy_choice.PersistentBusyInteraction, Protocol):
    @property
    def user(self) -> PersistentBusyUser: ...

    @property
    def channel_id(self) -> int | str | None: ...

    @property
    def response(self) -> PersistentBusyResponse: ...


class PersistentBusyChannel(discord_persistent_busy_choice.PersistentBusyChannel, Protocol):
    @property
    def id(self) -> int | str | None: ...


class BusyComponentClearer(Protocol):
    def __call__(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        *,
        context: str,
    ) -> Awaitable[None]: ...


class BusyResponseSender(Protocol):
    def __call__(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        content: str,
        *,
        ephemeral: bool,
        context: str,
    ) -> Awaitable[None]: ...


class BusySimpleResponseSender(Protocol):
    def __call__(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        content: str,
        *,
        context: str,
    ) -> Awaitable[None]: ...


class BusyDirectFollowupSender(Protocol):
    def __call__(
        self,
        interaction: discord_persistent_busy_choice.PersistentBusyInteraction,
        content: str,
        *,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


class InteractionChannelResolver(Protocol):
    def __call__(
        self,
        interaction: PersistentBusyInteraction,
        channel_id: int,
    ) -> Awaitable[PersistentBusyChannel | None]: ...


@dataclass(frozen=True, slots=True)
class PersistentBusyChoiceInteractionDeps:
    get_busy_choice_record: BusyChoiceRecordGetter
    claim_busy_choice_record: BusyChoiceRecordClaimer
    clear_interaction_message_components: BusyComponentClearer
    send_interaction_response: BusyResponseSender
    clear_busy_interaction_components: BusyComponentClearer
    send_busy_interaction_response: BusySimpleResponseSender
    send_busy_direct_followup: BusyDirectFollowupSender
    resolve_interaction_channel: InteractionChannelResolver
    steer_action_deps: discord_persistent_busy_steer_action.PersistentBusySteerActionDeps
    queue_action_deps: discord_persistent_busy_queue.PersistentBusyQueueActionDeps
    stop_action_deps: discord_busy_choice_stop_action.BusyChoiceStopActionDeps
    log: LogFunc


async def handle_persistent_busy_choice_interaction(
    interaction: PersistentBusyInteraction,
    custom_id: str,
    *,
    deps: PersistentBusyChoiceInteractionDeps,
) -> bool:
    user_id = int(str(interaction.user.id or "0"))
    resolution = discord_persistent_busy_choice.resolve_persistent_busy_choice(
        custom_id,
        user_id=user_id,
        get_busy_choice_record=deps.get_busy_choice_record,
    )
    if resolution.status == "unhandled":
        return False

    choice_id = resolution.choice_id
    action = resolution.action
    if resolution.status == "missing":
        await deps.clear_interaction_message_components(interaction, context="busy_choice_missing")
        await deps.send_interaction_response(
            interaction,
            "This Discord button is no longer active. Send the message again to get fresh controls.",
            ephemeral=True,
            context="busy_choice_missing",
        )
        deps.log(f"busy_choice_persistent_missing action={action} choice={choice_id} channel={interaction.channel_id} user={user_id}")
        return True

    record = resolution.record
    if record is None:
        return True

    target_thread_id_for_log = resolution.target_thread_id or "-"
    if resolution.status == "denied":
        await deps.send_interaction_response(
            interaction,
            "Only the original sender can choose this.",
            ephemeral=True,
            context="busy_choice_persistent_denied",
        )
        denied_log = " ".join(
            [
                f"busy_choice_persistent_denied action={action} choice={choice_id}",
                f"user={user_id} owner={resolution.owner_user_id} target={target_thread_id_for_log}",
            ]
        )
        deps.log(denied_log)
        return True

    if resolution.status == "steer_not_allowed":
        await deps.send_interaction_response(
            interaction,
            "This message targets a different Codex thread. Queue it instead.",
            ephemeral=True,
            context="busy_choice_persistent_steer_rejected",
        )
        deps.log(f"busy_choice_persistent_steer_rejected user={user_id} choice={choice_id} target={target_thread_id_for_log} reason=not_allowed")
        return True

    if not deps.claim_busy_choice_record(choice_id):
        await deps.clear_interaction_message_components(interaction, context="busy_choice_already_handled")
        await deps.send_interaction_response(
            interaction,
            "This busy choice was already handled.",
            ephemeral=True,
            context="busy_choice_persistent_already_handled",
        )
        deps.log(f"busy_choice_persistent_already_handled action={action} choice={choice_id} user={user_id} target={target_thread_id_for_log}")
        return True

    prompt = str(record["prompt"] or "")
    target_thread_id = discord_persistent_busy_choice.normalize_record_thread_id(record)
    if action == "ignore":
        return await discord_persistent_busy_choice.handle_persistent_busy_ignore(
            interaction,
            user_id=user_id,
            choice_id=choice_id,
            target_thread_id=target_thread_id,
            deps=discord_persistent_busy_choice.PersistentBusyIgnoreDeps(
                clear_components=deps.clear_busy_interaction_components,
                send_response=deps.send_busy_interaction_response,
                log=deps.log,
            ),
        )

    _ = await interaction.response.defer(
        thinking=action != "queue",
        ephemeral=action in {"steer", "stop"},
    )
    await deps.clear_interaction_message_components(interaction, context=f"busy_choice_{action}")
    channel = await deps.resolve_interaction_channel(
        interaction,
        discord_persistent_busy_choice.normalize_record_channel_id(record),
    )
    if channel is None:
        return await discord_persistent_busy_choice.handle_persistent_busy_channel_unavailable(
            interaction,
            action=action,
            choice_id=choice_id,
            target_thread_id=target_thread_id,
            deps=discord_persistent_busy_choice.PersistentBusyChannelUnavailableDeps(
                send_followup=deps.send_busy_direct_followup,
                log=deps.log,
            ),
        )

    source_message = discord_persistent_busy_choice.make_persistent_busy_source_message(record, channel)
    if action == "steer":
        return await discord_persistent_busy_steer_action.handle_persistent_busy_steer_action(
            interaction,
            channel,
            user_id=user_id,
            choice_id=choice_id,
            target_thread_id=target_thread_id,
            prompt=prompt,
            deps=deps.steer_action_deps,
        )

    if action == "stop":
        return await discord_busy_choice_stop_action.handle_busy_choice_stop_action(
            interaction,
            channel,
            target_thread_id,
            user_id=user_id,
            deps=deps.stop_action_deps,
        )

    return await discord_persistent_busy_queue.handle_persistent_busy_queue_action(
        interaction,
        channel,
        source_message,
        user_id=user_id,
        choice_id=choice_id,
        target_thread_id=target_thread_id,
        prompt=prompt,
        deps=deps.queue_action_deps,
    )
