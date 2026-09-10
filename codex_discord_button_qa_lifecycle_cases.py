from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass
from typing import Protocol, TypeAlias

from codex_discord_components import ComponentRowLike

STALE_BUTTON_MESSAGE = "This Discord button is no longer active. Send the message again to get fresh controls."


class QaResponseLike(Protocol):
    @property
    def messages(self) -> list[str]:
        ...


class BusyChoiceQaInteraction(Protocol):
    @property
    def response(self) -> QaResponseLike:
        ...


class LifecycleQaBot(Protocol):
    pass


class LifecycleQaChannel(Protocol):
    pass


class LifecycleQaUser(Protocol):
    pass


class LifecycleQaMessage(Protocol):
    @property
    def components(self) -> Iterable[ComponentRowLike] | None:
        ...


class BusyChoiceRecord(Protocol):
    pass


SendCaseButtonResult: TypeAlias = tuple[LifecycleQaMessage, dict[str, str], str]


class SendCaseButtonFunc(Protocol):
    def __call__(self, prompt: str) -> Awaitable[SendCaseButtonResult]:
        ...


class MakeInteractionFunc(Protocol):
    def __call__(
        self,
        *,
        bot: LifecycleQaBot,
        channel: LifecycleQaChannel,
        message: LifecycleQaMessage,
        user: LifecycleQaUser,
        custom_id: str,
    ) -> BusyChoiceQaInteraction:
        ...


class BusyChoiceInteractionHandler(Protocol):
    def __call__(self, interaction: BusyChoiceQaInteraction, custom_id: str) -> Awaitable[bool]:
        ...


class StaleCleanupFunc(Protocol):
    def __call__(self, message: LifecycleQaMessage) -> Awaitable[bool]:
        ...


@dataclass(frozen=True, slots=True)
class BusyChoiceLifecycleQaCaseDeps:
    send_case_button: SendCaseButtonFunc
    make_interaction: MakeInteractionFunc
    handle_persistent_busy_choice_interaction: BusyChoiceInteractionHandler
    claim_busy_choice_record: Callable[[str], bool]
    get_busy_choice_record: Callable[[str], BusyChoiceRecord | None]
    delete_busy_choice_record: Callable[[str], None]
    clear_stale_busy_choice_message_components: StaleCleanupFunc


async def run_busy_choice_lifecycle_qa_cases(
    *,
    bot: LifecycleQaBot,
    channel: LifecycleQaChannel,
    user: LifecycleQaUser,
    deps: BusyChoiceLifecycleQaCaseDeps,
) -> list[str]:
    return [
        await _run_ignore_case(bot=bot, channel=channel, user=user, deps=deps),
        await _run_claimed_record_case(bot=bot, channel=channel, user=user, deps=deps),
        await _run_missing_record_case(bot=bot, channel=channel, user=user, deps=deps),
        await _run_stale_cleanup_case(deps=deps),
    ]


async def _run_ignore_case(
    *,
    bot: LifecycleQaBot,
    channel: LifecycleQaChannel,
    user: LifecycleQaUser,
    deps: BusyChoiceLifecycleQaCaseDeps,
) -> str:
    sent_message, custom_ids, choice_id = await deps.send_case_button("QA button ignore smoke")
    interaction = deps.make_interaction(
        bot=bot,
        channel=channel,
        message=sent_message,
        user=user,
        custom_id=custom_ids["Ignore"],
    )
    handled = await deps.handle_persistent_busy_choice_interaction(interaction, custom_ids["Ignore"])
    record_cleared = deps.get_busy_choice_record(choice_id) is None
    return "ignore: " + (
        "ok" if handled and record_cleared and interaction.response.messages == ["Ignored."] else "failed"
    )


async def _run_claimed_record_case(
    *,
    bot: LifecycleQaBot,
    channel: LifecycleQaChannel,
    user: LifecycleQaUser,
    deps: BusyChoiceLifecycleQaCaseDeps,
) -> str:
    sent_message, custom_ids, choice_id = await deps.send_case_button("QA button claimed-record smoke")
    _claimed = deps.claim_busy_choice_record(choice_id)
    interaction = deps.make_interaction(
        bot=bot,
        channel=channel,
        message=sent_message,
        user=user,
        custom_id=custom_ids["Queue next"],
    )
    handled = await deps.handle_persistent_busy_choice_interaction(interaction, custom_ids["Queue next"])
    return "claimed_record: " + (
        "ok" if handled and interaction.response.messages == [STALE_BUTTON_MESSAGE] else "failed"
    )


async def _run_missing_record_case(
    *,
    bot: LifecycleQaBot,
    channel: LifecycleQaChannel,
    user: LifecycleQaUser,
    deps: BusyChoiceLifecycleQaCaseDeps,
) -> str:
    sent_message, custom_ids, choice_id = await deps.send_case_button("QA button missing-record smoke")
    deps.delete_busy_choice_record(choice_id)
    interaction = deps.make_interaction(
        bot=bot,
        channel=channel,
        message=sent_message,
        user=user,
        custom_id=custom_ids["Steer now"],
    )
    handled = await deps.handle_persistent_busy_choice_interaction(interaction, custom_ids["Steer now"])
    return "missing_record: " + (
        "ok" if handled and interaction.response.messages == [STALE_BUTTON_MESSAGE] else "failed"
    )


async def _run_stale_cleanup_case(*, deps: BusyChoiceLifecycleQaCaseDeps) -> str:
    sent_message, _custom_ids, choice_id = await deps.send_case_button("QA button stale cleanup smoke")
    deps.delete_busy_choice_record(choice_id)
    cleanup_done = await deps.clear_stale_busy_choice_message_components(sent_message)
    return "stale_cleanup: " + (
        "ok" if cleanup_done and deps.get_busy_choice_record(choice_id) is None else "failed"
    )
