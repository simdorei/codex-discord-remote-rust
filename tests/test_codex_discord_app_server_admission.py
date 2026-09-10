from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import unittest

from codex_app_server_transport_lifecycle import (
    AppServerGenerationExpiredError,
    AppServerLifecycleSnapshot,
)
import codex_discord_app_server_admission as admission


@dataclass(frozen=True)
class Message:
    id: int
    created_at: datetime | None


@dataclass(frozen=True)
class SyntheticInteractionMessage:
    channel: object
    author: object


@dataclass(frozen=True)
class TimestampedSyntheticMessage:
    created_at: datetime


class Client:
    def __init__(self, snapshot: AppServerLifecycleSnapshot) -> None:
        self.snapshot = snapshot

    def lifecycle_snapshot(self) -> AppServerLifecycleSnapshot:
        return self.snapshot


class AppServerAdmissionTests(unittest.IsolatedAsyncioTestCase):
    async def test_message_created_before_generation_is_discarded(self) -> None:
        notices: list[str] = []
        logs: list[str] = []
        snapshot = AppServerLifecycleSnapshot(generation=2, healthy=True, accepting_since=200.0)
        message = Message(id=101, created_at=datetime.fromtimestamp(199.0, tz=timezone.utc))

        async def send_notice(_channel: object, text: str) -> object:
            notices.append(text)
            return object()

        async with admission.admit_prompt_delivery(
            object(),
            message,
            expected_generation=None,
            transport_enabled=True,
            client=Client(snapshot),
            send_notice=send_notice,
            log=logs.append,
        ) as accepted:
            self.assertFalse(accepted)

        self.assertEqual(len(notices), 1)
        self.assertIn("reason=source_predates_generation", logs[0])
        self.assertIsNone(admission.current_expected_app_server_generation())

    async def test_timestamped_synthetic_source_before_generation_is_discarded(self) -> None:
        notices: list[str] = []
        logs: list[str] = []
        snapshot = AppServerLifecycleSnapshot(generation=2, healthy=True, accepting_since=200.0)
        message = TimestampedSyntheticMessage(
            created_at=datetime.fromtimestamp(199.0, tz=timezone.utc)
        )

        async def send_notice(_channel: object, text: str) -> object:
            notices.append(text)
            return object()

        async with admission.admit_prompt_delivery(
            object(),
            message,
            expected_generation=None,
            transport_enabled=True,
            client=Client(snapshot),
            send_notice=send_notice,
            log=logs.append,
        ) as accepted:
            self.assertFalse(accepted)

        self.assertEqual(len(notices), 1)
        self.assertIn("reason=source_predates_generation", logs[0])

    async def test_current_message_sets_and_resets_expected_generation(self) -> None:
        seen: list[int | None] = []
        snapshot = AppServerLifecycleSnapshot(generation=3, healthy=True, accepting_since=200.0)
        message = Message(id=102, created_at=datetime.fromtimestamp(201.0, tz=timezone.utc))

        async def send_notice(_channel: object, _text: str) -> object:
            raise AssertionError("notice not expected")

        async with admission.admit_prompt_delivery(
            object(),
            message,
            expected_generation=None,
            transport_enabled=True,
            client=Client(snapshot),
            send_notice=send_notice,
            log=lambda _text: None,
        ) as accepted:
            self.assertTrue(accepted)
            seen.append(admission.current_expected_app_server_generation())

        self.assertEqual(seen, [3])
        self.assertIsNone(admission.current_expected_app_server_generation())

    async def test_explicit_stale_generation_raises_without_notice(self) -> None:
        snapshot = AppServerLifecycleSnapshot(generation=4, healthy=True, accepting_since=200.0)

        async def send_notice(_channel: object, _text: str) -> object:
            raise AssertionError("notice not expected")

        with self.assertRaises(AppServerGenerationExpiredError):
            async with admission.admit_prompt_delivery(
                object(),
                None,
                expected_generation=3,
                transport_enabled=True,
                client=Client(snapshot),
                send_notice=send_notice,
                log=lambda _text: None,
            ):
                self.fail("stale generation must not enter prompt delivery")

    async def test_unhealthy_server_discards_synthetic_user_source(self) -> None:
        notices: list[str] = []
        snapshot = AppServerLifecycleSnapshot(generation=4, healthy=False, accepting_since=None)

        async def send_notice(_channel: object, text: str) -> object:
            notices.append(text)
            return object()

        async with admission.admit_prompt_delivery(
            object(),
            SyntheticInteractionMessage(channel=object(), author=object()),
            expected_generation=None,
            transport_enabled=True,
            client=Client(snapshot),
            send_notice=send_notice,
            log=lambda _text: None,
        ) as accepted:
            self.assertFalse(accepted)

        self.assertEqual(len(notices), 1)

    async def test_internal_source_none_keeps_legacy_autostart_path(self) -> None:
        snapshot = AppServerLifecycleSnapshot(generation=4, healthy=False, accepting_since=None)

        async def send_notice(_channel: object, _text: str) -> object:
            raise AssertionError("internal compatibility path must not send a notice")

        async with admission.admit_prompt_delivery(
            object(),
            None,
            expected_generation=None,
            transport_enabled=True,
            client=Client(snapshot),
            send_notice=send_notice,
            log=lambda _text: None,
        ) as accepted:
            self.assertTrue(accepted)
            self.assertIsNone(admission.current_expected_app_server_generation())

    async def test_queued_source_predating_generation_raises_terminal_expiry(self) -> None:
        snapshot = AppServerLifecycleSnapshot(generation=5, healthy=True, accepting_since=200.0)
        message = Message(id=103, created_at=datetime.fromtimestamp(199.0, tz=timezone.utc))

        async def send_notice(_channel: object, _text: str) -> object:
            raise AssertionError("queued expiry is handled by the queue runner")

        with self.assertRaises(AppServerGenerationExpiredError):
            async with admission.admit_prompt_delivery(
                object(),
                message,
                expected_generation=5,
                transport_enabled=True,
                client=Client(snapshot),
                send_notice=send_notice,
                log=lambda _text: None,
            ):
                self.fail("queued stale source must not enter prompt delivery")


if __name__ == "__main__":
    _ = unittest.main()
