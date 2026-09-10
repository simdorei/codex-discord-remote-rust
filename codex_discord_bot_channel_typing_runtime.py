from __future__ import annotations

from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Protocol, TypeAlias

import codex_discord_approval_followup as discord_approval_followup
import codex_discord_prefix_steer_command as discord_prefix_steer_command

TypingContext: TypeAlias = AbstractAsyncContextManager[None]


class MessageableChannel(Protocol):
    pass


class MessageableChannelTyping(Protocol):
    def __call__(
        self,
        channel: MessageableChannel,
        *,
        context: str,
    ) -> TypingContext: ...


class MessageableChannelResolver(Protocol):
    def __call__(
        self,
        channel: discord_approval_followup.ApprovalFollowupChannel
        | discord_prefix_steer_command.ChannelLike,
    ) -> MessageableChannel: ...


class SteeringHandoffMarker(Protocol):
    def __call__(self, target_thread_id: str | None) -> float: ...


@dataclass(frozen=True, slots=True)
class BotChannelTypingRuntime:
    channel_typing_factory: Callable[[], MessageableChannelTyping]
    messageable_channel_resolver: Callable[[], MessageableChannelResolver]
    steering_handoff_marker: Callable[[], SteeringHandoffMarker]

    def mapped_prompt_delivery_channel_typing(
        self,
        channel: MessageableChannel,
        *,
        context: str,
    ) -> TypingContext:
        return self.channel_typing_factory()(channel, context=context)

    def prompt_delivery_channel_typing(
        self,
        channel: MessageableChannel,
        *,
        context: str,
    ) -> TypingContext:
        return self.channel_typing_factory()(channel, context=context)

    def approval_followup_channel_typing(
        self,
        channel: discord_approval_followup.ApprovalFollowupChannel,
        *,
        context: str,
    ) -> TypingContext:
        return self.channel_typing_factory()(
            self.messageable_channel_resolver()(channel),
            context=context,
        )

    def prefix_steer_channel_typing(
        self,
        channel: discord_prefix_steer_command.ChannelLike,
        *,
        context: str = "typing",
    ) -> TypingContext:
        return self.channel_typing_factory()(
            self.messageable_channel_resolver()(channel),
            context=context,
        )

    def mark_optional_steering_handoff(self, target_thread_id: str | None) -> None:
        _ = self.steering_handoff_marker()(target_thread_id)
