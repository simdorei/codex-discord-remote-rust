from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

import codex_discord_busy as discord_busy
from codex_discord_bot_prompt_delivery_types import PromptAdmission
import codex_discord_plain_ask as discord_plain_ask
import codex_discord_plain_ask_handler as discord_plain_ask_handler
import codex_discord_plain_ask_runtime as discord_plain_ask_runtime
import codex_discord_prompt_flow as discord_prompt_flow


MessageableT = TypeVar("MessageableT")
MessageableCoT = TypeVar("MessageableCoT", covariant=True)
MessageableContraT = TypeVar("MessageableContraT", contravariant=True)
MessageT = TypeVar("MessageT", bound=discord_plain_ask_runtime.PlainBusyAskMessage)
MessageContraT = TypeVar(
    "MessageContraT",
    bound=discord_plain_ask_runtime.PlainBusyAskMessage,
    contravariant=True,
)
ViewT = TypeVar("ViewT")
ViewCoT = TypeVar("ViewCoT", covariant=True)
ViewContraT = TypeVar("ViewContraT", contravariant=True)
SendResultT = TypeVar("SendResultT")
SendResultCoT = TypeVar("SendResultCoT", covariant=True)


class MessageableResolver(Protocol[MessageableCoT]):
    def __call__(self, channel: discord_plain_ask.PlainAskChannel) -> MessageableCoT: ...


class MessageSender(Protocol[MessageableContraT, ViewContraT, SendResultCoT]):
    def __call__(
        self,
        target: MessageableContraT,
        content: str,
        *,
        view: ViewContraT,
        context: str,
    ) -> Awaitable[SendResultCoT]: ...


class ChunkSender(Protocol[MessageableContraT, SendResultCoT]):
    def __call__(
        self,
        target: MessageableContraT,
        text: str,
        *,
        context: str | None = None,
    ) -> Awaitable[SendResultCoT]: ...


class ThreadAskEnqueuer(Protocol[MessageableContraT, MessageContraT]):
    def __call__(
        self,
        channel: MessageableContraT,
        prompt: str,
        target_thread_id: str | None,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: MessageContraT | None = None,
    ) -> Awaitable[int]: ...


class PlainPromptFlowRunner(Protocol[MessageableContraT, MessageContraT]):
    def __call__(
        self,
        channel: MessageableContraT,
        prompt: str,
        *,
        source_message: MessageContraT,
        target_thread_id: str | None,
    ) -> Awaitable[None]: ...


class PromptSender(Protocol[MessageableContraT, MessageContraT]):
    def __call__(
        self,
        channel: MessageableContraT,
        prompt: str,
        *,
        queued: bool = False,
        ack_sent: bool = False,
        source_message: MessageContraT | None = None,
        target_thread_id: str | None = None,
    ) -> Awaitable[None]: ...


class BusyChoiceViewFactory(Protocol[MessageContraT, ViewCoT]):
    def __call__(
        self,
        source_message: MessageContraT,
        prompt: str,
        *,
        target_thread_id: str | None,
        allow_steer: bool = True,
    ) -> ViewCoT: ...


@dataclass(frozen=True, slots=True)
class BotPlainAskRuntimeDeps(Generic[MessageableT, MessageT, ViewT, SendResultT]):
    discord_max_len: int
    fit_single_message: discord_busy.FitSingleMessage
    require_messageable_channel: MessageableResolver[MessageableT]
    make_busy_choice_view: BusyChoiceViewFactory[MessageT, ViewT]
    send_message_tracked: MessageSender[MessageableT, ViewT, SendResultT]
    send_chunks: ChunkSender[MessageableT, SendResultT]
    send_prompt_chunks: discord_prompt_flow.ChunkSender[MessageableT]
    enqueue_thread_ask: ThreadAskEnqueuer[MessageableT, MessageT]
    run_prompt_and_send: PromptSender[MessageableT, MessageT]
    prompt_admission: PromptAdmission[MessageableT]
    run_plain_prompt_flow: PlainPromptFlowRunner[MessageableT, MessageT]
    build_context_warning: Callable[[str | None], str]
    build_ask_start_message: discord_prompt_flow.QueuedAskStartBuilder
    get_interactive_state_for_thread: discord_plain_ask.GetInteractiveStateFunc
    send_interactive_prompt: discord_plain_ask.SendInteractivePromptFunc
    submit_interactive_reply: discord_plain_ask.SubmitInteractiveReplyFunc
    state_input: str
    state_approval: str
    has_recent_codex_app_user_prompt: discord_plain_ask_handler.HasRecentPromptSyncFunc
    is_thread_runner_busy: discord_plain_ask.IsRunnerBusyFunc
    mark_recent_discord_origin_prompt: discord_plain_ask.MarkRecentPromptFunc
    claim_direct_ask_target: discord_plain_ask.ClaimDirectAskTargetFunc
    release_direct_ask_target: discord_plain_ask.ReleaseDirectAskTargetFunc
    format_log_text_len: Callable[[str | None], int | str]
    format_log_text_len_as_text: discord_busy.FormatLogTextLen
    log: Callable[[str], None]
