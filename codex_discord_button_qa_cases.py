from __future__ import annotations

from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass
from typing import Protocol

from codex_discord_components import ComponentRowLike


class ButtonQaAuthor(Protocol):
    @property
    def bot(self) -> bool: ...

    @property
    def id(self) -> int: ...


class ButtonQaChannel(Protocol):
    @property
    def id(self) -> int | str | None: ...


class ButtonQaMessage(Protocol):
    @property
    def channel(self) -> ButtonQaChannel: ...

    @property
    def author(self) -> ButtonQaAuthor: ...
class ViewChildLike(Protocol):
    label: str | None
    custom_id: str | None


class SentMessageLike(Protocol):
    @property
    def components(self) -> Iterable[ComponentRowLike] | None: ...


class ViewLike(Protocol):
    @property
    def children(self) -> Iterable[ViewChildLike]:
        ...


class MakeBusyChoicePayloadFunc(Protocol):
    def __call__(
        self,
        source_message: ButtonQaMessage,
        prompt: str,
        *,
        target_thread_id: str | None = None,
        allow_steer: bool = True,
    ) -> tuple[str, ViewLike]:
        ...


class SendMessageTrackedFunc(Protocol):
    def __call__(
        self,
        channel: ButtonQaChannel,
        content: str,
        *,
        view: ViewLike | None = None,
        context: str = "send_message_tracked",
    ) -> Awaitable[SentMessageLike]:
        ...


@dataclass(frozen=True, slots=True)
class BusyChoiceQaCaseDeps:
    get_mirrored_codex_thread_id: Callable[[int | None], str | None]
    make_busy_choice_payload: MakeBusyChoicePayloadFunc
    send_message_tracked: SendMessageTrackedFunc
    parse_busy_choice_custom_id: Callable[[str], tuple[str, str] | None]
    is_button: Callable[[ViewChildLike], bool]


@dataclass(frozen=True, slots=True)
class BusyChoiceQaCase:
    sent_message: SentMessageLike
    view: ViewLike
    custom_ids: dict[str, str]
    choice_id: str


async def send_busy_choice_qa_case(
    message: ButtonQaMessage,
    channel: ButtonQaChannel,
    prompt: str,
    *,
    deps: BusyChoiceQaCaseDeps,
) -> BusyChoiceQaCase:
    content, view = deps.make_busy_choice_payload(
        message,
        prompt,
        target_thread_id=deps.get_mirrored_codex_thread_id(_channel_id(channel)),
        allow_steer=True,
    )
    sent_message = await deps.send_message_tracked(
        channel,
        content,
        view=view,
        context="button_qa_busy_choice",
    )
    custom_ids = {
        str(getattr(item, "label", "")): str(getattr(item, "custom_id", ""))
        for item in view.children
        if deps.is_button(item)
    }
    choice_id, _action = deps.parse_busy_choice_custom_id(custom_ids["Ignore"]) or ("", "")
    return BusyChoiceQaCase(
        sent_message=sent_message,
        view=view,
        custom_ids=custom_ids,
        choice_id=choice_id,
    )


def _channel_id(channel: ButtonQaChannel) -> int | None:
    value = getattr(channel, "id", None)
    return value if isinstance(value, int) else None
