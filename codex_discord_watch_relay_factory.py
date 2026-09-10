from __future__ import annotations

from typing import Protocol

import codex_discord_approval_followup as approval_followup
import codex_discord_steering_watch as steering_watch


class SteeringRelayFactory(Protocol):
    def __call__(
        self,
        loop: steering_watch.SteeringWatchLoop,
        channel: steering_watch.SteeringWatchChannel,
        target_thread_id: str,
        target_ref: str,
        *,
        suppress_after_steering_since: float,
        send_timeout_blocks: bool,
        send_commentary_blocks: bool | None,
        send_final_blocks: bool,
    ) -> steering_watch.SteeringWatchRelay: ...


class ApprovalFollowupRelayFactory(Protocol):
    def __call__(
        self,
        loop: approval_followup.ApprovalFollowupLoop,
        channel: approval_followup.ApprovalFollowupChannel,
        target_thread_id: str,
        target_ref: str,
        *,
        send_timeout_blocks: bool,
    ) -> approval_followup.ApprovalFollowupRelay: ...


def make_steering_watch_relay(
    relay_factory: SteeringRelayFactory,
    loop: steering_watch.SteeringWatchLoop,
    channel: steering_watch.SteeringWatchChannel,
    target_thread_id: str,
    target_ref: str,
    *,
    started_at: float,
    send_commentary_blocks: bool | None,
    send_final_blocks: bool,
) -> steering_watch.SteeringWatchRelay:
    return relay_factory(
        loop,
        channel,
        target_thread_id,
        target_ref,
        suppress_after_steering_since=started_at,
        send_timeout_blocks=False,
        send_commentary_blocks=send_commentary_blocks,
        send_final_blocks=send_final_blocks,
    )


def make_approval_followup_relay(
    relay_factory: ApprovalFollowupRelayFactory,
    loop: approval_followup.ApprovalFollowupLoop,
    channel: approval_followup.ApprovalFollowupChannel,
    target_thread_id: str,
    target_ref: str,
) -> approval_followup.ApprovalFollowupRelay:
    return relay_factory(
        loop,
        channel,
        target_thread_id,
        target_ref,
        send_timeout_blocks=False,
    )
