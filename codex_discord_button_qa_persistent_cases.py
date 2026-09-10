from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass
from typing import Protocol

Submitter = Callable[[str, str], tuple[int, str]]


class PersistentQaBot(Protocol):
    pass


class PersistentQaChannel(Protocol):
    pass


class PersistentQaUser(Protocol):
    pass


class PersistentQaMessage(Protocol):
    pass


class ViewChildLike(Protocol):
    @property
    def label(self) -> str | None:
        ...

    @property
    def custom_id(self) -> str | None:
        ...


class ViewLike(Protocol):
    @property
    def children(self) -> Iterable[ViewChildLike]:
        ...


class QaResponseLike(Protocol):
    @property
    def deferred(self) -> bool:
        ...


class QaFollowupLike(Protocol):
    @property
    def messages(self) -> list[str]:
        ...


class PersistentQaInteraction(Protocol):
    @property
    def response(self) -> QaResponseLike:
        ...

    @property
    def followup(self) -> QaFollowupLike:
        ...


class MakeInputChoiceViewFunc(Protocol):
    def __call__(self, target_thread_id: str, options: list[tuple[str, str]]) -> ViewLike:
        ...


class MakeInteractionFunc(Protocol):
    def __call__(
        self,
        *,
        bot: PersistentQaBot,
        channel: PersistentQaChannel,
        message: PersistentQaMessage,
        user: PersistentQaUser,
        custom_id: str,
    ) -> PersistentQaInteraction:
        ...


class SendMessageTrackedFunc(Protocol):
    def __call__(
        self,
        channel: PersistentQaChannel,
        content: str,
        *,
        view: ViewLike | None = None,
        context: str = "send_message_tracked",
    ) -> Awaitable[PersistentQaMessage]:
        ...


class ApprovalInteractionHandler(Protocol):
    def __call__(
        self,
        interaction: PersistentQaInteraction,
        custom_id: str,
        *,
        approval_submitter: Submitter,
    ) -> Awaitable[bool]:
        ...


class InputChoiceInteractionHandler(Protocol):
    def __call__(
        self,
        interaction: PersistentQaInteraction,
        custom_id: str,
        *,
        input_submitter: Submitter,
    ) -> Awaitable[bool]:
        ...


@dataclass(frozen=True, slots=True)
class PersistentButtonQaCaseDeps:
    make_approval_view: Callable[[str], ViewLike]
    make_input_choice_view: MakeInputChoiceViewFunc
    make_interaction: MakeInteractionFunc
    send_message_tracked: SendMessageTrackedFunc
    handle_persistent_approval_interaction: ApprovalInteractionHandler
    handle_persistent_input_choice_interaction: InputChoiceInteractionHandler
    is_button: Callable[[ViewChildLike], bool]


async def run_persistent_button_qa_cases(
    *,
    bot: PersistentQaBot,
    channel: PersistentQaChannel,
    user: PersistentQaUser,
    deps: PersistentButtonQaCaseDeps,
) -> list[str]:
    approval_line = await _run_approval_case(bot=bot, channel=channel, user=user, deps=deps)
    input_line = await _run_input_choice_case(bot=bot, channel=channel, user=user, deps=deps)
    return [approval_line, input_line]


async def _run_approval_case(
    *,
    bot: PersistentQaBot,
    channel: PersistentQaChannel,
    user: PersistentQaUser,
    deps: PersistentButtonQaCaseDeps,
) -> str:
    approval_view = deps.make_approval_view("qa-thread")
    approval_message = await deps.send_message_tracked(
        channel,
        "QA approval persistent smoke",
        view=approval_view,
        context="button_qa_approval",
    )
    approval_custom_ids = _custom_ids_by_label(approval_view, is_button=deps.is_button)
    approval_interaction = deps.make_interaction(
        bot=bot,
        channel=channel,
        message=approval_message,
        user=user,
        custom_id=approval_custom_ids["Approve session"],
    )
    approval_submitted: list[tuple[str, str]] = []

    def fake_submit_approval(target_thread_id: str, answer: str) -> tuple[int, str]:
        approval_submitted.append((target_thread_id, answer))
        return 0, "approved"

    approval_handled = await deps.handle_persistent_approval_interaction(
        approval_interaction,
        approval_custom_ids["Approve session"],
        approval_submitter=fake_submit_approval,
    )
    return (
        "approval_persistent: "
        + (
            "ok"
            if approval_handled
            and approval_interaction.response.deferred
            and approval_interaction.followup.messages == ["Approval submitted\n\napproved"]
            and approval_submitted == [("qa-thread", "2")]
            else "failed"
        )
    )


async def _run_input_choice_case(
    *,
    bot: PersistentQaBot,
    channel: PersistentQaChannel,
    user: PersistentQaUser,
    deps: PersistentButtonQaCaseDeps,
) -> str:
    input_view = deps.make_input_choice_view("qa-thread", [("choice-1", "Choice one")])
    input_message = await deps.send_message_tracked(
        channel,
        "QA input persistent smoke",
        view=input_view,
        context="button_qa_input",
    )
    input_custom_ids = _custom_ids_by_label(input_view, is_button=deps.is_button)
    input_interaction = deps.make_interaction(
        bot=bot,
        channel=channel,
        message=input_message,
        user=user,
        custom_id=input_custom_ids["Choice one"],
    )
    input_submitted: list[tuple[str, str]] = []

    def fake_submit_input(target_thread_id: str, value: str) -> tuple[int, str]:
        input_submitted.append((target_thread_id, value))
        return 0, "answered"

    input_handled = await deps.handle_persistent_input_choice_interaction(
        input_interaction,
        input_custom_ids["Choice one"],
        input_submitter=fake_submit_input,
    )
    return (
        "input_choice_persistent: "
        + (
            "ok"
            if input_handled
            and input_interaction.response.deferred
            and input_interaction.followup.messages == ["Input submitted\n\nanswered"]
            and input_submitted == [("qa-thread", "choice-1")]
            else "failed"
        )
    )


def _custom_ids_by_label(
    view: ViewLike,
    *,
    is_button: Callable[[ViewChildLike], bool],
) -> dict[str, str]:
    return {
        str(item.label): str(item.custom_id)
        for item in view.children
        if is_button(item)
    }
