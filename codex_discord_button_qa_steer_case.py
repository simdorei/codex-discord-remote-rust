from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Protocol, TypeAlias, override

import codex_discord_persistent_busy_steer as discord_persistent_busy_steer
import codex_discord_persistent_busy_choice_interaction as discord_persistent_busy_choice_interaction
from codex_discord_steering import SteeringPromptResult

ChannelIdValue: TypeAlias = int | str | None
DeferKwargValue: TypeAlias = str | int | float | bool | None
DeferKwargs: TypeAlias = dict[str, DeferKwargValue]


class SteerQaBot(Protocol):
    pass


class SteerQaChannel(Protocol):
    @property
    def id(self) -> ChannelIdValue:
        ...


class SteerQaUser(discord_persistent_busy_choice_interaction.PersistentBusyUser, Protocol):
    pass


class SteerQaMessage(Protocol):
    pass


class QaResponseLike(discord_persistent_busy_choice_interaction.PersistentBusyResponse, Protocol):
    deferred: bool
    defer_kwargs: list[DeferKwargs]


class QaFollowupLike(Protocol):
    messages: list[str]


class SteerQaInteraction(discord_persistent_busy_choice_interaction.PersistentBusyInteraction, Protocol):
    @property
    @override
    def response(self) -> QaResponseLike:
        ...

    @property
    def followup(self) -> QaFollowupLike:
        ...

    @property
    @override
    def user(self) -> SteerQaUser:
        ...


class SendCaseButtonFunc(Protocol):
    def __call__(self, prompt: str) -> Awaitable[tuple[SteerQaMessage, dict[str, str], str]]:
        ...


class MakeInteractionFunc(Protocol):
    def __call__(
        self,
        *,
        bot: SteerQaBot,
        channel: SteerQaChannel,
        message: SteerQaMessage,
        user: SteerQaUser,
        custom_id: str,
    ) -> SteerQaInteraction:
        ...


SteeringRunner: TypeAlias = discord_persistent_busy_steer.SteeringRunner
SteeringStreamer: TypeAlias = discord_persistent_busy_steer.PersistentBusySteerStreamer


class BusyChoiceSteerHandler(Protocol):
    def __call__(
        self,
        interaction: SteerQaInteraction,
        custom_id: str,
        *,
        steering_runner: SteeringRunner,
        steering_streamer: SteeringStreamer,
    ) -> Awaitable[bool]:
        ...


@dataclass(frozen=True, slots=True)
class BusyChoiceSteerQaCaseDeps:
    send_case_button: SendCaseButtonFunc
    make_interaction: MakeInteractionFunc
    handle_persistent_busy_choice_interaction: BusyChoiceSteerHandler
    delete_busy_choice_record: Callable[[str], None]
    get_mirrored_codex_thread_id: Callable[[int | None], str | None]
    make_steering_prompt_result: Callable[[str | None], SteeringPromptResult]


async def run_busy_choice_steer_success_qa_case(
    *,
    bot: SteerQaBot,
    channel: SteerQaChannel,
    user: SteerQaUser,
    deps: BusyChoiceSteerQaCaseDeps,
) -> str:
    sent_message, custom_ids, choice_id = await deps.send_case_button("QA button steer success smoke")
    interaction = deps.make_interaction(
        bot=bot,
        channel=channel,
        message=sent_message,
        user=user,
        custom_id=custom_ids["Steer now"],
    )
    watched: list[tuple[str | None, str | None]] = []

    def fake_run_steering_prompt(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
        _ = prompt
        return deps.make_steering_prompt_result(target_thread_id)

    async def fake_stream_steering_prompt_result_to_channel(
        channel: discord_persistent_busy_steer.PersistentBusyChannel,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> None:
        _ = (channel, send_commentary_blocks, send_final_blocks)
        watched.append((target_thread_id, _target_thread_id(steering_result)))

    handled = await deps.handle_persistent_busy_choice_interaction(
        interaction,
        custom_ids["Steer now"],
        steering_runner=fake_run_steering_prompt,
        steering_streamer=fake_stream_steering_prompt_result_to_channel,
    )
    deps.delete_busy_choice_record(choice_id)
    target_thread_id = deps.get_mirrored_codex_thread_id(_channel_id(channel))
    return "steer_success: " + (
        "ok"
        if handled
        and interaction.response.deferred
        and interaction.response.defer_kwargs
        and interaction.response.defer_kwargs[-1].get("ephemeral") is True
        and interaction.followup.messages
        and str(interaction.followup.messages[0]).startswith("Steering sent")
        and watched == [(target_thread_id, target_thread_id)]
        else "failed"
    )


def _channel_id(channel: SteerQaChannel) -> int | None:
    value = channel.id
    return value if isinstance(value, int) else None


def _target_thread_id(steering_result: SteeringPromptResult) -> str | None:
    value = steering_result.target_thread_id
    return value if isinstance(value, str) or value is None else None
