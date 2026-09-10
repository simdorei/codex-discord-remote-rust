from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from dataclasses import dataclass
import unittest
from unittest.mock import patch

import codex_discord_bot as bot
import codex_discord_approval_followup as approval_followup
import codex_discord_steering_watch as steering_watch
import codex_discord_watch_relay_factory as watch_relay_factory


@dataclass(slots=True)  # noqa: MUTABLE_OK - approval relay protocol exposes writable status flags.
class FakeRelay:
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False
    suppressed_after_steering: bool = False


@dataclass(frozen=True, slots=True)
class FakeChannel:
    name: str


@dataclass(frozen=True, slots=True)
class SteeringRelayCall:
    loop: steering_watch.SteeringWatchLoop
    channel: steering_watch.SteeringWatchChannel
    target_thread_id: str
    target_ref: str
    suppress_after_steering_since: float
    send_timeout_blocks: bool
    send_commentary_blocks: bool | None
    send_final_blocks: bool


@dataclass(frozen=True, slots=True)
class ApprovalFollowupRelayCall:
    loop: approval_followup.ApprovalFollowupLoop
    channel: approval_followup.ApprovalFollowupChannel
    target_thread_id: str
    target_ref: str
    send_timeout_blocks: bool


class WatchRelayFactoryTests(unittest.TestCase):
    def test_make_steering_watch_relay_forwards_existing_flags(self) -> None:
        # Given
        calls: list[SteeringRelayCall] = []
        relay = FakeRelay(sent_live=True)
        loop = asyncio.new_event_loop()
        self.addCleanup(loop.close)
        channel = FakeChannel("steer")

        def relay_factory(
            loop: steering_watch.SteeringWatchLoop,
            channel: steering_watch.SteeringWatchChannel,
            target_thread_id: str,
            target_ref: str,
            *,
            suppress_after_steering_since: float,
            send_timeout_blocks: bool,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> FakeRelay:
            calls.append(
                SteeringRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id=target_thread_id,
                    target_ref=target_ref,
                    suppress_after_steering_since=suppress_after_steering_since,
                    send_timeout_blocks=send_timeout_blocks,
                    send_commentary_blocks=send_commentary_blocks,
                    send_final_blocks=send_final_blocks,
                )
            )
            return relay

        # When
        made = watch_relay_factory.make_steering_watch_relay(
            relay_factory,
            loop,
            channel,
            "thread-1",
            "project:1",
            started_at=42.5,
            send_commentary_blocks=False,
            send_final_blocks=True,
        )

        # Then
        self.assertIs(made, relay)
        self.assertEqual(
            calls,
            [
                SteeringRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id="thread-1",
                    target_ref="project:1",
                    suppress_after_steering_since=42.5,
                    send_timeout_blocks=False,
                    send_commentary_blocks=False,
                    send_final_blocks=True,
                )
            ],
        )

    def test_make_approval_followup_relay_forwards_existing_flags(self) -> None:
        # Given
        calls: list[ApprovalFollowupRelayCall] = []
        relay = FakeRelay(sent_live=True)
        loop = asyncio.new_event_loop()
        self.addCleanup(loop.close)
        channel = FakeChannel("approval")

        def relay_factory(
            loop: approval_followup.ApprovalFollowupLoop,
            channel: approval_followup.ApprovalFollowupChannel,
            target_thread_id: str,
            target_ref: str,
            *,
            send_timeout_blocks: bool,
        ) -> FakeRelay:
            calls.append(
                ApprovalFollowupRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id=target_thread_id,
                    target_ref=target_ref,
                    send_timeout_blocks=send_timeout_blocks,
                )
            )
            return relay

        # When
        made = watch_relay_factory.make_approval_followup_relay(
            relay_factory,
            loop,
            channel,
            "thread-2",
            "project:2",
        )

        # Then
        self.assertIs(made, relay)
        self.assertEqual(
            calls,
            [
                ApprovalFollowupRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id="thread-2",
                    target_ref="project:2",
                    send_timeout_blocks=False,
                )
            ],
        )

    def test_bot_make_steering_watch_relay_uses_current_relay_factory(self) -> None:
        # Given
        calls: list[SteeringRelayCall] = []
        relay = FakeRelay(sent_live=True)
        loop = asyncio.new_event_loop()
        self.addCleanup(loop.close)
        channel = FakeChannel("bot-steer")

        def relay_factory(
            loop: steering_watch.SteeringWatchLoop,
            channel: steering_watch.SteeringWatchChannel,
            target_thread_id: str,
            target_ref: str,
            *,
            suppress_after_steering_since: float,
            send_timeout_blocks: bool,
            send_commentary_blocks: bool | None,
            send_final_blocks: bool,
        ) -> FakeRelay:
            calls.append(
                SteeringRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id=target_thread_id,
                    target_ref=target_ref,
                    suppress_after_steering_since=suppress_after_steering_since,
                    send_timeout_blocks=send_timeout_blocks,
                    send_commentary_blocks=send_commentary_blocks,
                    send_final_blocks=send_final_blocks,
                )
            )
            return relay

        # When
        with patch.object(bot, "DiscordAskRelay", relay_factory):
            made = bot.make_steering_watch_relay(
                loop,
                channel,
                "thread-3",
                "project:3",
                started_at=77.25,
                send_commentary_blocks=True,
                send_final_blocks=False,
            )

        # Then
        self.assertIs(made, relay)
        self.assertEqual(
            calls,
            [
                SteeringRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id="thread-3",
                    target_ref="project:3",
                    suppress_after_steering_since=77.25,
                    send_timeout_blocks=False,
                    send_commentary_blocks=True,
                    send_final_blocks=False,
                )
            ],
        )

    def test_bot_make_approval_followup_relay_uses_current_relay_factory(self) -> None:
        # Given
        calls: list[ApprovalFollowupRelayCall] = []
        relay = FakeRelay(sent_live=True)
        loop = asyncio.new_event_loop()
        self.addCleanup(loop.close)
        channel = FakeChannel("bot-approval")

        def relay_factory(
            loop: approval_followup.ApprovalFollowupLoop,
            channel: approval_followup.ApprovalFollowupChannel,
            target_thread_id: str,
            target_ref: str,
            *,
            send_timeout_blocks: bool,
        ) -> FakeRelay:
            calls.append(
                ApprovalFollowupRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id=target_thread_id,
                    target_ref=target_ref,
                    send_timeout_blocks=send_timeout_blocks,
                )
            )
            return relay

        # When
        with patch.object(bot, "DiscordAskRelay", relay_factory):
            made = bot.make_approval_followup_relay(
                loop,
                channel,
                "thread-4",
                "project:4",
            )

        # Then
        self.assertIs(made, relay)
        self.assertEqual(
            calls,
            [
                ApprovalFollowupRelayCall(
                    loop=loop,
                    channel=channel,
                    target_thread_id="thread-4",
                    target_ref="project:4",
                    send_timeout_blocks=False,
                )
            ],
        )


if __name__ == "__main__":
    _ = unittest.main()
