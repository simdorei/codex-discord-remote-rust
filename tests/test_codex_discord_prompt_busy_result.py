from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
import unittest

import codex_discord_prompt_busy_result as busy_result
from codex_thread_models import ThreadInfo


@dataclass(slots=True)
class FakeChannel:
    messages: list[str] = field(default_factory=list)


@dataclass(slots=True)
class BusyResultFixture:
    recorded_result: bool = False
    app_menu_result: bool = False
    recorded_calls: list[tuple[str, str | None, str, bool]] = field(default_factory=list)
    settle_calls: list[tuple[str, str | None]] = field(default_factory=list)
    handoffs: list[str] = field(default_factory=list)
    app_menu_calls: list[tuple[str | None, str, str]] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)

    async def handle_recorded(
        self,
        channel: FakeChannel,
        prompt: str,
        *,
        target_thread_id: str | None,
        target_ref: str,
        recent_offsets: dict[str, tuple[ThreadInfo, Path, int]],
        transport_output: str,
        delegate_to_session_mirror: bool,
    ) -> bool:
        _ = channel, recent_offsets, transport_output
        self.recorded_calls.append((prompt, target_thread_id, target_ref, delegate_to_session_mirror))
        return self.recorded_result

    async def wait_settle(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        recent_offsets: dict[str, tuple[ThreadInfo, Path, int]],
    ) -> None:
        _ = recent_offsets
        self.settle_calls.append((prompt, target_thread_id))

    async def send_app_menu(
        self,
        channel: FakeChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        _ = channel
        self.app_menu_calls.append((target_thread_id, output, reason))
        return self.app_menu_result

    def build(self) -> busy_result.AskStreamBusyResultDeps[FakeChannel]:
        return busy_result.AskStreamBusyResultDeps(
            handle_recorded_busy_transport_prompt=self.handle_recorded,
            wait_for_mirrored_busy_delegation_settle=self.wait_settle,
            mark_steering_handoff=self.handoffs.append,
            send_codex_app_menu_if_available=self.send_app_menu,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class AskStreamBusyResultTests(unittest.IsolatedAsyncioTestCase):
    def test_busy_retry_message_includes_target_and_attempts(self) -> None:
        output = busy_result.build_codex_app_busy_retry_message("thread-1", 3)

        self.assertIn("Codex app did not accept this Discord message yet.", output)
        self.assertIn("target: `thread-1`", output)
        self.assertIn("retry_attempts: 3", output)

    def test_steering_not_accepted_message_includes_target(self) -> None:
        output = busy_result.build_codex_app_steering_not_accepted_message("thread-1")

        self.assertIn("Codex app did not accept this steering message yet.", output)
        self.assertIn("target: `thread-1`", output)

    async def test_recorded_delivery_returns_handled(self) -> None:
        fixture = BusyResultFixture(recorded_result=True)

        handled = await busy_result.handle_ask_stream_busy_result(
            FakeChannel(),
            "please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            recent_offsets={},
            transport_output="busy",
            delegate_to_session_mirror=False,
            retry_index=None,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.recorded_calls, [("please run", "thread-1", "taxlab:1", False)])
        self.assertEqual(fixture.app_menu_calls, [])

    async def test_delegate_to_session_mirror_marks_handoff_and_settles(self) -> None:
        fixture = BusyResultFixture()

        handled = await busy_result.handle_ask_stream_busy_result(
            FakeChannel(),
            "please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            recent_offsets={},
            transport_output="busy",
            delegate_to_session_mirror=True,
            retry_index=None,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.handoffs, ["thread-1"])
        self.assertEqual(fixture.settle_calls, [("please run", "thread-1")])
        self.assertIn("ask_stream_busy_delegated_to_session_mirror target=thread-1", "\n".join(fixture.logs))

    async def test_app_menu_initial_reason_returns_handled(self) -> None:
        fixture = BusyResultFixture(app_menu_result=True)

        handled = await busy_result.handle_ask_stream_busy_result(
            FakeChannel(),
            "please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            recent_offsets={},
            transport_output="busy",
            delegate_to_session_mirror=False,
            retry_index=None,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.app_menu_calls, [("thread-1", "busy", "ask_target_busy_failure")])

    async def test_app_menu_retry_reason_returns_handled(self) -> None:
        fixture = BusyResultFixture(app_menu_result=True)

        handled = await busy_result.handle_ask_stream_busy_result(
            FakeChannel(),
            "please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            recent_offsets={},
            transport_output="busy",
            delegate_to_session_mirror=False,
            retry_index=2,
            deps=fixture.build(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.app_menu_calls, [("thread-1", "busy", "ask_busy_retry_2")])

    async def test_returns_not_handled_when_no_busy_side_effect_applies(self) -> None:
        fixture = BusyResultFixture()

        handled = await busy_result.handle_ask_stream_busy_result(
            FakeChannel(),
            "please run",
            target_thread_id="thread-1",
            target_ref="taxlab:1",
            recent_offsets={},
            transport_output="busy",
            delegate_to_session_mirror=False,
            retry_index=None,
            deps=fixture.build(),
        )

        self.assertFalse(handled)
