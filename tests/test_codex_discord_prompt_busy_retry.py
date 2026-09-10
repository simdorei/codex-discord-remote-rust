from __future__ import annotations

import unittest
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from dataclasses import dataclass
from pathlib import Path

import codex_discord_prompt_busy_result as prompt_busy_result
import codex_discord_prompt_busy_retry as busy_retry
import codex_discord_prompt_pending_delivery as pending_delivery
import codex_discord_prompt_retry_attempt as retry_attempt
import codex_discord_prompt_retry_exhausted as retry_exhausted
import codex_discord_prompt_retry_suppression as retry_suppression
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class FakeRelay:
    name: str
    sent_live: bool = False
    saw_final: bool = False
    saw_aborted: bool = False
    saw_timeout: bool = False
    suppressed_after_steering: bool = False
    relay_generation: int = 0


@dataclass(frozen=True, slots=True)
class FakeRetryResult:
    exit_code: int
    output: str
    relay: FakeRelay


class BusyRetryFlowTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        busy_outputs: set[str] | None = None,
        busy_result_handles: bool = False,
        pending_handles: bool = False,
        suppresses_retry: bool = False,
        retry_result: FakeRetryResult | None = None,
    ) -> tuple[busy_retry.BusyRetryFlowDeps[str, FakeRelay, None], list[str]]:
        events: list[str] = []
        busy_markers = busy_outputs or {"busy"}

        async def sleep(delay: float) -> None:
            events.append(f"sleep:{delay}")

        def make_retry_relay(
            channel: str,
            *,
            target_thread_id: str | None,
            target_ref: str,
            started_at: float,
            delegate_to_session_mirror: bool,
        ) -> FakeRelay:
            events.append(f"relay:{channel}:{target_thread_id}:{target_ref}:{started_at}:{delegate_to_session_mirror}")
            relay = retry_result.relay if retry_result else FakeRelay("retry")
            if suppresses_retry:
                return FakeRelay(relay.name, suppressed_after_steering=True)
            return relay

        @asynccontextmanager
        async def channel_typing(channel: str, *, context: str) -> AsyncGenerator[None, None]:
            events.append(f"typing:{channel}:{context}")
            yield

        async def run_ask_stream(
            prompt: str,
            relay: FakeRelay,
            *,
            target_thread_id: str | None,
        ) -> tuple[int, str]:
            events.append(f"retry:{prompt}:{relay.name}:{target_thread_id}")
            result = retry_result or FakeRetryResult(0, "ok", relay)
            return result.exit_code, result.output

        async def send_pending_chunks(
            channel: str,
            content: str,
            *,
            context: str | None = None,
        ) -> None:
            events.append(f"pending_send:{channel}:{context}:{content}")

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
            events.append(f"recorded_busy:{channel}:{prompt}:{target_thread_id}:{target_ref}:{len(recent_offsets)}:{transport_output}:{delegate_to_session_mirror}")
            return busy_result_handles

        async def wait_for_settle(
            prompt: str,
            *,
            target_thread_id: str | None,
            recent_offsets: prompt_busy_result.RecentOffsets,
        ) -> None:
            events.append(f"settle:{prompt}:{target_thread_id}:{len(recent_offsets)}")

        async def send_app_menu(channel: str, target_thread_id: str | None, output: str, *, reason: str) -> bool:
            events.append(f"app_menu:{channel}:{target_thread_id}:{output}:{reason}")
            return False

        async def send_chunks(channel: str, text: str) -> None:
            events.append(f"send:{channel}:{text}")

        deps = busy_retry.BusyRetryFlowDeps(
            is_selected_thread_busy_error=lambda exit_code, output: exit_code != 0 and output in busy_markers,
            retry_attempt_deps=retry_attempt.RetryAttemptDeps(
                sleep=sleep,
                make_retry_relay=make_retry_relay,
                channel_typing=channel_typing,
                run_ask_stream=run_ask_stream,
                format_log_text_len=lambda text: len(text or ""),
                log=lambda text: events.append(f"log:{text}"),
            ),
            retry_suppression_deps=retry_suppression.RetrySuppressionDeps(
                is_discord_relay_stale=lambda target, generation: False,
                format_log_text_len=lambda text: len(text or ""),
                log=lambda text: events.append(f"log:{text}"),
            ),
            pending_delivery_deps=pending_delivery.AskStreamPendingDeliveryDeps(
                is_delivery_confirmation_timeout=lambda output: pending_handles,
                send_chunks=send_pending_chunks,
                format_log_text_len=lambda text: len(text or ""),
                log=lambda text: events.append(f"log:{text}"),
            ),
            busy_result_deps=prompt_busy_result.AskStreamBusyResultDeps(
                handle_recorded_busy_transport_prompt=handle_recorded_busy,
                wait_for_mirrored_busy_delegation_settle=wait_for_settle,
                mark_steering_handoff=lambda target: events.append(f"mark:{target}"),
                send_codex_app_menu_if_available=send_app_menu,
                format_log_text_len=lambda text: len(text or ""),
                log=lambda text: events.append(f"log:{text}"),
            ),
            retry_exhausted_deps=retry_exhausted.RetryExhaustedDeps(
                is_selected_thread_busy_error=lambda exit_code, output: exit_code != 0 and output in busy_markers,
                build_codex_app_busy_retry_message=lambda target_ref, attempts: f"exhausted:{target_ref}:{attempts}",
                send_chunks=send_pending_chunks,
                format_log_text_len=lambda text: len(text or ""),
                log=lambda text: events.append(f"log:{text}"),
            ),
            send_chunks=send_chunks,
            log=lambda text: events.append(f"log:{text}"),
        )
        return deps, events

    def recent_offsets(self) -> busy_retry.RecentOffsets:
        thread = ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:/repo",
            updated_at=1,
            rollout_path="session.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )
        return {"thread-1": (thread, Path("session.jsonl"), 0)}

    async def test_non_busy_result_is_returned_unhandled(self) -> None:
        deps, events = self.make_deps()

        result = await busy_retry.handle_busy_retry_flow(
            "channel",
            prompt="prompt",
            exit_code=0,
            output="ok",
            relay=FakeRelay("initial"),
            target_thread_id="thread-1",
            target_ref="project:1",
            recent_offsets=self.recent_offsets(),
            delegate_to_session_mirror=False,
            started_at=10,
            retry_attempts=1,
            retry_delay=0,
            source_message_available=False,
            deps=deps,
        )

        self.assertFalse(result.handled)
        self.assertEqual(result.output, "ok")
        self.assertEqual(events, [])

    async def test_initial_busy_result_can_handle_without_retry(self) -> None:
        deps, events = self.make_deps(busy_result_handles=True)

        result = await busy_retry.handle_busy_retry_flow(
            "channel",
            prompt="prompt",
            exit_code=1,
            output="busy",
            relay=FakeRelay("initial"),
            target_thread_id="thread-1",
            target_ref="project:1",
            recent_offsets=self.recent_offsets(),
            delegate_to_session_mirror=True,
            started_at=10,
            retry_attempts=1,
            retry_delay=0,
            source_message_available=True,
            deps=deps,
        )

        self.assertTrue(result.handled)
        self.assertIn("log:ask_stream_busy_transport_failure kind=target target=thread-1 source_message=yes", events)
        self.assertIn("recorded_busy:channel:prompt:thread-1:project:1:1:busy:True", events)
        self.assertFalse(any(event.startswith("retry:") for event in events))

    async def test_busy_result_retries_until_non_busy(self) -> None:
        deps, events = self.make_deps(retry_result=FakeRetryResult(0, "ok", FakeRelay("retry")))

        result = await busy_retry.handle_busy_retry_flow(
            "channel",
            prompt="prompt",
            exit_code=1,
            output="busy",
            relay=FakeRelay("initial"),
            target_thread_id="thread-1",
            target_ref="project:1",
            recent_offsets=self.recent_offsets(),
            delegate_to_session_mirror=False,
            started_at=10,
            retry_attempts=1,
            retry_delay=0.25,
            source_message_available=False,
            deps=deps,
        )

        self.assertFalse(result.handled)
        self.assertEqual(result.exit_code, 0)
        self.assertEqual(result.output, "ok")
        self.assertIn("send:channel:Codex app did not accept this Discord message yet. Retrying mapped-thread delivery up to 1 time(s).", events)
        self.assertIn("sleep:0.25", events)
        self.assertIn("relay:channel:thread-1:project:1:10:False", events)
        self.assertIn("retry:prompt:retry:thread-1", events)

    async def test_busy_result_reports_exhaustion(self) -> None:
        deps, events = self.make_deps()

        result = await busy_retry.handle_busy_retry_flow(
            "channel",
            prompt="prompt",
            exit_code=1,
            output="busy",
            relay=FakeRelay("initial"),
            target_thread_id="thread-1",
            target_ref="project:1",
            recent_offsets=self.recent_offsets(),
            delegate_to_session_mirror=False,
            started_at=10,
            retry_attempts=0,
            retry_delay=0,
            source_message_available=False,
            deps=deps,
        )

        self.assertTrue(result.handled)
        self.assertIn("pending_send:channel:None:exhausted:project:1:0", events)


if __name__ == "__main__":
    unittest.main()
