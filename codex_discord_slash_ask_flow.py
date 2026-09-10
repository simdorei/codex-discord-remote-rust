from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar


class SlashAskUser(Protocol):
    @property
    def id(self) -> int: ...


ChannelT = TypeVar("ChannelT")
ChannelT_contra = TypeVar("ChannelT_contra", contravariant=True)
ChannelT_co = TypeVar("ChannelT_co", covariant=True)
UserT = TypeVar("UserT", bound=SlashAskUser)
UserT_contra = TypeVar("UserT_contra", bound=SlashAskUser, contravariant=True)
UserT_co = TypeVar("UserT_co", bound=SlashAskUser, covariant=True)
SourceMessageT = TypeVar("SourceMessageT")
SourceMessageT_contra = TypeVar("SourceMessageT_contra", contravariant=True)
SourceMessageT_co = TypeVar("SourceMessageT_co", covariant=True)


class SlashAskInteraction(Protocol[ChannelT_co, UserT_co]):
    @property
    def channel(self) -> ChannelT_co | None: ...

    @property
    def channel_id(self) -> int | None: ...

    @property
    def user(self) -> UserT_co: ...


class SlashAskChunkSender(Protocol[ChannelT_contra, UserT_contra]):
    def __call__(
        self,
        interaction: SlashAskInteraction[ChannelT_contra, UserT_contra],
        text: str,
        *,
        title: str,
    ) -> Awaitable[None]: ...


class SlashAskFollowupSender(Protocol[ChannelT_contra, UserT_contra]):
    def __call__(
        self,
        interaction: SlashAskInteraction[ChannelT_contra, UserT_contra],
        text: str,
        *,
        ephemeral: bool,
        log_prefix: str,
        context: str,
    ) -> Awaitable[None]: ...


class SlashAskHandler(Protocol[SourceMessageT_contra]):
    def __call__(
        self,
        source_message: SourceMessageT_contra,
        prompt: str,
        *,
        target_thread_id: str | None = None,
    ) -> Awaitable[None]: ...


class SourceMessageFactory(Protocol[ChannelT_contra, UserT_contra, SourceMessageT_co]):
    def __call__(self, channel: ChannelT_contra, user: UserT_contra) -> SourceMessageT_co: ...


@dataclass(frozen=True, slots=True)
class SlashAskFlowDeps(Generic[ChannelT, UserT, SourceMessageT]):
    send_interaction_chunks: SlashAskChunkSender[ChannelT, UserT]
    send_direct_followup: SlashAskFollowupSender[ChannelT, UserT]
    handle_plain_ask: SlashAskHandler[SourceMessageT]
    get_mirrored_thread_id: Callable[[int | None], str | None]
    describe_project_channel: Callable[[int | None], str | None]
    get_command_name: Callable[[SlashAskInteraction[ChannelT, UserT]], str]
    format_text_len: Callable[[str], int]
    is_messageable_channel: Callable[[ChannelT], bool]
    make_source_message: SourceMessageFactory[ChannelT, UserT, SourceMessageT]
    log: Callable[[str], None]


async def handle_slash_ask(
    interaction: SlashAskInteraction[ChannelT, UserT],
    prompt: str,
    *,
    deps: SlashAskFlowDeps[ChannelT, UserT, SourceMessageT],
) -> None:
    channel = interaction.channel
    if channel is None or not deps.is_messageable_channel(channel):
        await deps.send_interaction_chunks(
            interaction,
            "This Discord interaction has no messageable channel.",
            title="Ask",
        )
        return

    target_thread_id = deps.get_mirrored_thread_id(interaction.channel_id)
    target_source = "mirror" if target_thread_id else "selected"
    if target_thread_id is None:
        project_message = deps.describe_project_channel(interaction.channel_id)
        if project_message:
            deps.log(
                f"slash_ask_blocked command={deps.get_command_name(interaction)} "
                + f"channel={interaction.channel_id} user={interaction.user.id} "
                + f"reason=project_parent prompt_len={deps.format_text_len(prompt)}"
            )
            await deps.send_interaction_chunks(interaction, project_message, title="Ask")
            return

    deps.log(
        f"slash_ask_dispatch command={deps.get_command_name(interaction)} "
        + f"channel={interaction.channel_id} user={interaction.user.id} "
        + f"target_source={target_source} target={target_thread_id or '-'} "
        + f"prompt_len={deps.format_text_len(prompt)}"
    )
    await deps.send_direct_followup(
        interaction,
        "Ask handling posted in this channel.",
        ephemeral=True,
        log_prefix="slash_ack",
        context="ask_posted",
    )
    deps.log(
        f"slash_ask_ack_sent command={deps.get_command_name(interaction)} "
        + f"channel={interaction.channel_id}"
    )
    await deps.handle_plain_ask(
        deps.make_source_message(channel, interaction.user),
        prompt,
        target_thread_id=target_thread_id,
    )
