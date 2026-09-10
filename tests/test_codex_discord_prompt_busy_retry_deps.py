from __future__ import annotations

from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from dataclasses import dataclass
import unittest

import codex_discord_prompt_busy_result as prompt_busy_result
import codex_discord_prompt_busy_retry as busy_retry
import codex_discord_prompt_pending_delivery as pending_delivery


@dataclass(frozen=True, slots=True)
class FakeRelay:
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False
    suppressed_after_steering: bool = False
    relay_generation: int = 0


class BusyRetryDepsFactoryTests(unittest.TestCase):
    def test_make_busy_retry_flow_deps_wires_nested_deps(self) -> None:
        async def sleep(delay: float) -> None:
            self.assertEqual(delay, 0.25)

        def make_retry_relay(
            channel: str,
            *,
            target_thread_id: str | None,
            target_ref: str,
            started_at: float,
            delegate_to_session_mirror: bool,
        ) -> FakeRelay:
            self.assertEqual((channel, target_thread_id, target_ref, started_at, delegate_to_session_mirror), ("c", "t", "r", 1.0, True))
            return FakeRelay()

        @asynccontextmanager
        async def channel_typing(channel: str, *, context: str) -> AsyncGenerator[None, None]:
            self.assertEqual((channel, context), ("c", "ask_stream_retry"))
            yield

        async def run_ask_stream(prompt: str, relay: FakeRelay, *, target_thread_id: str | None) -> tuple[int, str]:
            self.assertEqual((prompt, target_thread_id, relay.sent_live), ("p", "t", False))
            return 0, "ok"

        async def send_pending(channel: str, content: str, *, context: str | None = None) -> None:
            self.assertEqual((channel, content, context), ("c", "pending", None))

        async def handle_recorded_busy(
            channel: str,
            prompt: str,
            *,
            target_thread_id: str | None,
            target_ref: str,
            recent_offsets: prompt_busy_result.RecentOffsets,
            transport_output: str,
            delegate_to_session_mirror: bool,
        ) -> bool:
            self.assertEqual((channel, prompt, target_thread_id, target_ref), ("c", "p", "t", "r"))
            self.assertEqual((len(recent_offsets), transport_output, delegate_to_session_mirror), (0, "busy", False))
            return False

        async def wait_for_settle(
            prompt: str,
            *,
            target_thread_id: str | None,
            recent_offsets: prompt_busy_result.RecentOffsets,
        ) -> None:
            self.assertEqual((prompt, target_thread_id, len(recent_offsets)), ("p", "t", 0))

        async def send_app_menu(channel: str, target_thread_id: str | None, output: str, *, reason: str) -> bool:
            self.assertEqual((channel, target_thread_id, output, reason), ("c", "t", "busy", "busy"))
            return False

        async def send_notice(channel: str, text: str) -> None:
            self.assertEqual((channel, text), ("c", "notice"))

        pending_deps = pending_delivery.AskStreamPendingDeliveryDeps(
            is_delivery_confirmation_timeout=lambda output: output == "pending",
            send_chunks=send_pending,
            format_log_text_len=lambda text: len(text or ""),
            log=lambda message: None,
        )
        busy_deps = prompt_busy_result.AskStreamBusyResultDeps(
            handle_recorded_busy_transport_prompt=handle_recorded_busy,
            wait_for_mirrored_busy_delegation_settle=wait_for_settle,
            mark_steering_handoff=lambda target: None,
            send_codex_app_menu_if_available=send_app_menu,
            format_log_text_len=lambda text: len(text or ""),
            log=lambda message: None,
        )

        deps = busy_retry.make_busy_retry_flow_deps(
            is_selected_thread_busy_error=lambda exit_code, output: exit_code == 1 and output == "busy",
            sleep=sleep,
            make_retry_relay=make_retry_relay,
            channel_typing=channel_typing,
            run_ask_stream=run_ask_stream,
            is_discord_relay_stale=lambda target, generation: target == "t" and generation == 1,
            pending_delivery_deps=pending_deps,
            busy_result_deps=busy_deps,
            send_retry_notice_chunks=send_notice,
            send_status_chunks=send_pending,
            build_codex_app_busy_retry_message=lambda target_ref, attempts: f"retry:{target_ref}:{attempts}",
            format_log_text_len=lambda text: len(text or ""),
            log=lambda message: None,
        )

        self.assertIs(deps.pending_delivery_deps, pending_deps)
        self.assertIs(deps.busy_result_deps, busy_deps)
        self.assertTrue(deps.is_selected_thread_busy_error(1, "busy"))
        self.assertEqual(deps.retry_exhausted_deps.build_codex_app_busy_retry_message("r", 2), "retry:r:2")


if __name__ == "__main__":
    unittest.main()
