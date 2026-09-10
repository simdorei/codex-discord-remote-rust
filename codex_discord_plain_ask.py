from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Final, Generic, Protocol, TypeAlias, TypeVar, assert_never

import codex_discord_message_gate as discord_message_gate
import codex_discord_message_mentions as discord_message_mentions

InteractiveStateResult: TypeAlias = tuple[str, str | None, str]
FormatLogTextLenFunc: TypeAlias = Callable[[str], int | str]
HasRecentPromptFunc: TypeAlias = Callable[[str | None, str], Awaitable[bool]]
IsRunnerBusyFunc: TypeAlias = Callable[[str | None], Awaitable[bool]]
MarkRecentPromptFunc: TypeAlias = Callable[[str | None, str], None]
ClaimDirectAskTargetFunc: TypeAlias = Callable[[str | None], Awaitable[bool]]
ReleaseDirectAskTargetFunc: TypeAlias = Callable[[str | None], Awaitable[None]]

DUPLICATE_RECENT_APP_PROMPT_MESSAGE: Final = (
    "Already in Codex app. Skipping duplicate Discord delivery for this mapped thread."
)

MessageT = TypeVar("MessageT", bound="PlainAskMessage")
MessageContraT = TypeVar("MessageContraT", bound="PlainAskMessage", contravariant=True)
PreparedMessageT = TypeVar("PreparedMessageT", bound="PlainAskPreparedMessage")
PreparedMessageContraT = TypeVar(
    "PreparedMessageContraT",
    bound="PlainAskPreparedMessage",
    contravariant=True,
)
SendResultT = TypeVar("SendResultT")
SendResultCoT = TypeVar("SendResultCoT", covariant=True)


class PlainAskChannel(Protocol):
    pass


class PlainAskMessage(Protocol):
    @property
    def channel(self) -> PlainAskChannel: ...


class PlainAskAuthor(Protocol):
    @property
    def id(self) -> discord_message_mentions.DiscordIdValue: ...


class PlainAskPreparedMessage(discord_message_mentions.MessageWithMentions, PlainAskMessage, Protocol):
    @property
    def author(self) -> PlainAskAuthor: ...


GetInteractiveStateFunc: TypeAlias = Callable[[str | None], InteractiveStateResult]
NormalizeInteractiveReplyFunc: TypeAlias = Callable[[str, str], str | None]
SendInteractivePromptFunc: TypeAlias = Callable[[PlainAskChannel, str, str, str, str, list[str]], Awaitable[None]]
SubmitInteractiveReplyFunc: TypeAlias = Callable[[PlainAskChannel, str, str, str, str], Awaitable[None]]


class HandleBusyPlainAskFunc(Protocol[MessageContraT]):
    def __call__(
        self,
        message: MessageContraT,
        prompt: str,
        target_thread_id: str | None,
    ) -> Awaitable[None]: ...


class RunPromptFlowFunc(Protocol[MessageContraT]):
    def __call__(
        self,
        channel: PlainAskChannel,
        prompt: str,
        *,
        source_message: MessageContraT,
        target_thread_id: str | None,
    ) -> Awaitable[None]: ...


class SendChunksFunc(Protocol[SendResultCoT]):
    def __call__(
        self,
        channel: PlainAskChannel,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> Awaitable[SendResultCoT]: ...


class BuildPromptWithAttachmentsFunc(Protocol[PreparedMessageContraT]):
    def __call__(
        self,
        message: PreparedMessageContraT,
        prompt: str,
    ) -> Awaitable[str]: ...


@dataclass(frozen=True, slots=True)
class PlainAskInteractiveDeps:
    get_interactive_state_for_thread: GetInteractiveStateFunc
    normalize_interactive_text_reply: NormalizeInteractiveReplyFunc
    send_interactive_prompt: SendInteractivePromptFunc
    submit_interactive_reply: SubmitInteractiveReplyFunc
    state_input: str
    state_approval: str


@dataclass(frozen=True, slots=True)
class PlainAskInteractiveResult:
    handled: bool
    ask_target_thread_id: str | None


@dataclass(frozen=True, slots=True)
class PlainAskDirectDeps(Generic[MessageT, SendResultT]):
    has_recent_codex_app_user_prompt: HasRecentPromptFunc
    is_thread_runner_busy: IsRunnerBusyFunc
    mark_recent_discord_origin_prompt: MarkRecentPromptFunc
    handle_busy_plain_ask: HandleBusyPlainAskFunc[MessageT]
    claim_direct_ask_target: ClaimDirectAskTargetFunc
    release_direct_ask_target: ReleaseDirectAskTargetFunc
    run_prompt_flow: RunPromptFlowFunc[MessageT]
    send_chunks: SendChunksFunc[SendResultT]
    format_log_text_len: FormatLogTextLenFunc
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class PlainAskMessageContentDeps(Generic[PreparedMessageT, SendResultT]):
    build_prompt_with_discord_attachments: BuildPromptWithAttachmentsFunc[PreparedMessageT]
    send_chunks: SendChunksFunc[SendResultT]
    log: Callable[[str], None]


async def prepare_plain_ask_message_content(
    client: discord_message_mentions.DiscordClientWithMentions,
    message: PreparedMessageT,
    content: str,
    target_thread_id: str | None,
    *,
    has_attachments: bool,
    deps: PlainAskMessageContentDeps[PreparedMessageT, SendResultT],
) -> str | None:
    plain_ask_gate = discord_message_gate.prepare_plain_ask_content(
        message,
        content,
        discord_message_gate.get_bridge_mention_user_ids(client),
        target_thread_id,
        has_attachments=has_attachments,
    )
    prepared_content = plain_ask_gate.content
    channel_id = getattr(message.channel, "id", "-")
    author_id = message.author.id
    if plain_ask_gate.context_fallback:
        deps.log(f"plain_ask_context_fallback chat={channel_id} user={author_id}")
    action = plain_ask_gate.action
    if action == discord_message_gate.PlainAskGateAction.ACCEPT:
        return await deps.build_prompt_with_discord_attachments(message, prepared_content)
    if action == discord_message_gate.PlainAskGateAction.REQUIRED_MENTION_MISSING:
        deps.log(f"ignored_message reason=required_mention_missing chat={channel_id} user={author_id}")
        return None
    if action == discord_message_gate.PlainAskGateAction.MENTION_ONLY_CONTENT:
        deps.log(f"ignored_message reason=mention_only_content chat={channel_id} user={author_id}")
        _ = await deps.send_chunks(message.channel, "Add a prompt after the mention.")
        return None
    if action == discord_message_gate.PlainAskGateAction.OTHER_BOT_MENTION_IN_MIRRORED_THREAD:
        deps.log(f"ignored_message reason=other_bot_mention_in_mirrored_thread chat={channel_id} user={author_id}")
        return None
    assert_never(action)


async def handle_interactive_plain_ask(
    message: PlainAskMessage,
    prompt: str,
    target_thread_id: str | None,
    *,
    deps: PlainAskInteractiveDeps,
) -> PlainAskInteractiveResult:
    interactive_state, resolved_thread_id, target_ref = deps.get_interactive_state_for_thread(target_thread_id)
    ask_target_thread_id = target_thread_id or resolved_thread_id
    if not interactive_state or not resolved_thread_id:
        return PlainAskInteractiveResult(False, ask_target_thread_id)

    normalized_reply = deps.normalize_interactive_text_reply(interactive_state, prompt)
    if normalized_reply is None:
        prompt_text = "Pending approval" if interactive_state == deps.state_approval else "Pending input"
        await deps.send_interactive_prompt(
            message.channel,
            resolved_thread_id,
            target_ref,
            interactive_state,
            prompt_text,
            [],
        )
        return PlainAskInteractiveResult(True, ask_target_thread_id)

    await deps.submit_interactive_reply(
        message.channel,
        resolved_thread_id,
        target_ref,
        interactive_state,
        normalized_reply,
    )
    return PlainAskInteractiveResult(True, ask_target_thread_id)


async def handle_direct_plain_ask(
    message: MessageT,
    prompt: str,
    target_thread_id: str | None,
    *,
    deps: PlainAskDirectDeps[MessageT, SendResultT],
) -> None:
    if await deps.has_recent_codex_app_user_prompt(target_thread_id, prompt):
        prompt_len = deps.format_log_text_len(prompt)
        deps.log(
            f"plain_ask_duplicate_recent_app_prompt_skipped target={target_thread_id or '-'} prompt_len={prompt_len}"
        )
        _ = await deps.send_chunks(message.channel, DUPLICATE_RECENT_APP_PROMPT_MESSAGE)
        return

    if await deps.is_thread_runner_busy(target_thread_id):
        deps.mark_recent_discord_origin_prompt(target_thread_id, prompt)
        await deps.handle_busy_plain_ask(message, prompt, target_thread_id)
        return

    if not await deps.claim_direct_ask_target(target_thread_id):
        deps.mark_recent_discord_origin_prompt(target_thread_id, prompt)
        await deps.handle_busy_plain_ask(message, prompt, target_thread_id)
        return

    try:
        deps.mark_recent_discord_origin_prompt(target_thread_id, prompt)
        await deps.run_prompt_flow(
            message.channel,
            prompt,
            source_message=message,
            target_thread_id=target_thread_id,
        )
    finally:
        await deps.release_direct_ask_target(target_thread_id)
