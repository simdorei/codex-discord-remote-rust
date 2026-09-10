from __future__ import annotations

import asyncio
from dataclasses import dataclass, field
import threading
from types import TracebackType
import unittest

import codex_discord_steering as steering
import codex_discord_prompt_mapped_delivery as mapped_delivery
from codex_pro_runtime_diagnostics import (
    ProDiagnosticCode,
    ProDiagnosticStage,
    diagnostic,
)


@dataclass(slots=True)
class FakeChannel:
    messages: list[str] = field(default_factory=list)
    typing_events: list[str] = field(default_factory=list)


@dataclass(slots=True)
class FakeTypingContext:
    channel: FakeChannel

    async def __aenter__(self) -> None:
        self.channel.typing_events.append("enter")

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None:
        _ = exc_type, exc, traceback
        self.channel.typing_events.append("exit")
        return None


@dataclass(slots=True)
class DepsFixture:
    prepared: bool = True
    transport_result: tuple[int, str] = (0, "delivered")
    pending: bool = False
    busy: bool = False
    app_menu_result: bool = False
    preprocess_result: mapped_delivery.PromptPreprocessResult | None = None
    transport_calls: list[tuple[str, str | None]] = field(default_factory=list)
    marked_discord_origin_prompts: list[tuple[str | None, str]] = field(
        default_factory=list
    )
    deactivated: list[str | None] = field(default_factory=list)
    release_result: bool = True
    app_menu_calls: list[tuple[str | None, str, str]] = field(default_factory=list)
    resume_failures: list[tuple[str, str]] = field(default_factory=list)
    selected_thread_ids: list[str] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)
    preprocess_started: threading.Event | None = None
    preprocess_release: threading.Event | None = None

    async def prepare(self, channel: FakeChannel, target_thread_id: str | None) -> bool:
        _ = channel, target_thread_id
        return self.prepared

    def typing(self, channel: FakeChannel, *, context: str) -> FakeTypingContext:
        self.logs.append(f"typing_context={context}")
        return FakeTypingContext(channel)

    def preprocess(
        self,
        prompt: str,
        target_thread_id: str | None = None,
    ) -> mapped_delivery.PromptPreprocessResult:
        _ = target_thread_id
        if self.preprocess_started is not None and self.preprocess_release is not None:
            self.preprocess_started.set()
            released = self.preprocess_release.wait(timeout=0.5)
            self.logs.append(f"preprocess_released={released}")
        if self.preprocess_result is None:
            return mapped_delivery.keep_prompt(prompt)
        self.logs.append(f"preprocess_prompt={prompt}")
        return self.preprocess_result

    def mark_discord_origin_prompt(
        self, target_thread_id: str | None, prompt: str
    ) -> None:
        self.marked_discord_origin_prompts.append((target_thread_id, prompt))

    async def transport(
        self, prompt: str, target_thread_id: str | None
    ) -> tuple[int, str]:
        self.transport_calls.append((prompt, target_thread_id))
        return self.transport_result

    async def send_chunks(
        self,
        channel: FakeChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        _ = context
        channel.messages.append(content)

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

    async def send_resume_failure(
        self,
        channel: FakeChannel,
        content: str,
        target_thread_id: str,
    ) -> None:
        _ = channel
        self.resume_failures.append((content, target_thread_id))

    def set_selected_thread_id(self, thread_id: str) -> None:
        self.selected_thread_ids.append(thread_id)

    async def release(self, thread_id: str | None) -> bool:
        if self.release_result:
            self.deactivated.append(thread_id)
        return self.release_result

    def build(self) -> mapped_delivery.MappedPromptDeliveryDeps[FakeChannel]:
        return mapped_delivery.MappedPromptDeliveryDeps(
            prepare_mapped_session_mirror_output=self.prepare,
            set_selected_thread_id=self.set_selected_thread_id,
            channel_typing=self.typing,
            preprocess_prompt=self.preprocess,
            mark_recent_discord_origin_prompt=self.mark_discord_origin_prompt,
            run_transport_prompt_no_wait=self.transport,
            send_chunks=self.send_chunks,
            is_delivery_confirmation_timeout=lambda output: self.pending,
            format_pending_ask_delivery_output=lambda output: (
                f"[delivery_pending]\n{output}"
            ),
            release_session_mirror_output_target=self.release,
            is_selected_thread_busy_error=lambda exit_code, output: self.busy,
            send_codex_app_menu_if_available=self.send_app_menu,
            send_resume_failure=self.send_resume_failure,
            format_log_text_len=lambda output: len(output or ""),
            log=self.logs.append,
        )


class MappedPromptDeliveryTests(unittest.IsolatedAsyncioTestCase):
    async def test_blocked_preprocess_reports_reason_without_transport(self) -> None:
        fixture = DepsFixture(
            preprocess_result=mapped_delivery.block_prompt(
                diagnostic(
                    stage=ProDiagnosticStage.PLUGIN_INVENTORY,
                    code=ProDiagnosticCode.BROWSER_PLUGIN_DISABLED,
                    public_message="The Browser plugin is installed but disabled.",
                    recovery_action="Enable the Browser plugin, then retry !pro.",
                    internal_detail="private plugin path C:/secret/browser is disabled",
                )
            )
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "!pro inspect",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertFalse(result.accepted)
        self.assertIn("C:/secret/browser", result.error_message)
        self.assertEqual(len(channel.messages), 1)
        self.assertIn("installed but disabled", channel.messages[0])
        self.assertIn("Enable the Browser plugin", channel.messages[0])
        self.assertNotIn("C:/secret", channel.messages[0])
        self.assertEqual(fixture.transport_calls, [])
        self.assertEqual(fixture.marked_discord_origin_prompts, [])
        self.assertEqual(channel.typing_events, [])
        self.assertIn(
            "stage=plugin_inventory code=browser_plugin_disabled",
            "\n".join(fixture.logs),
        )

    async def test_blocking_preprocessor_does_not_block_discord_event_loop(
        self,
    ) -> None:
        started = threading.Event()
        release = threading.Event()
        fixture = DepsFixture(
            preprocess_started=started,
            preprocess_release=release,
        )
        loop = asyncio.get_running_loop()
        _ = loop.call_later(0.05, release.set)

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            FakeChannel(),
            "!pro inspect",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertTrue(started.is_set())
        self.assertIn("preprocess_released=True", fixture.logs)

    async def test_success_sends_visible_preprocess_line_and_delivers_rewritten_prompt(
        self,
    ) -> None:
        fixture = DepsFixture(
            transport_result=(0, "delivered"),
            preprocess_result=mapped_delivery.PromptPreprocessResult(
                prompt="Check Discord QA",
                visible_line="\ubc88\uc5ed: Check Discord QA",
            ),
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "$custom \ub514\uc2a4\ucf54\ub4dc QA \ud655\uc778",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(channel.messages, ["\ubc88\uc5ed: Check Discord QA"])
        self.assertEqual(fixture.transport_calls, [("Check Discord QA", "thread-1")])
        self.assertEqual(
            fixture.marked_discord_origin_prompts, [("thread-1", "Check Discord QA")]
        )

    async def test_prepare_false_returns_not_handled_without_transport(self) -> None:
        fixture = DepsFixture(prepared=False)
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertFalse(result.handled)
        self.assertEqual(fixture.transport_calls, [])
        self.assertEqual(channel.messages, [])
        self.assertEqual(channel.typing_events, [])

    async def test_success_uses_transport_and_logs_without_channel_message(
        self,
    ) -> None:
        fixture = DepsFixture(transport_result=(0, "delivered"))
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(fixture.transport_calls, [("please run", "thread-1")])
        self.assertEqual(channel.typing_events, ["enter", "exit"])
        self.assertEqual(channel.messages, [])
        self.assertEqual(fixture.deactivated, [])
        self.assertIn(
            "ask_transport_no_wait_done exit=0 target=thread-1", "\n".join(fixture.logs)
        )

    async def test_success_syncs_selected_thread_to_mapped_target(self) -> None:
        fixture = DepsFixture(transport_result=(0, "delivered"))
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(fixture.selected_thread_ids, ["thread-1"])

    async def test_pending_delivery_keeps_output_target_active(self) -> None:
        fixture = DepsFixture(
            transport_result=(1, "delivery could not be confirmed"),
            pending=True,
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(
            channel.messages, ["[delivery_pending]\ndelivery could not be confirmed"]
        )
        self.assertEqual(fixture.deactivated, [])

    async def test_exit_zero_pending_delivery_reports_pending_message(self) -> None:
        fixture = DepsFixture(
            transport_result=(0, "[delivery_pending]\nnot confirmed"),
            pending=True,
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(
            channel.messages, ["[delivery_pending]\n[delivery_pending]\nnot confirmed"]
        )
        self.assertEqual(fixture.deactivated, [])

    async def test_failure_deactivates_and_sends_transport_failure(self) -> None:
        fixture = DepsFixture(transport_result=(7, "boom"))
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(fixture.deactivated, ["thread-1"])
        self.assertEqual(channel.messages, ["Ask failed (transport exit 7)\n\nboom"])
        self.assertEqual(fixture.resume_failures, [])

    async def test_failure_preserves_output_target_when_subscription_release_is_deferred(
        self,
    ) -> None:
        fixture = DepsFixture(
            transport_result=(7, "boom"),
            release_result=False,
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(fixture.deactivated, [])
        self.assertIn("mapped_prompt_output_release_deferred", "\n".join(fixture.logs))
        self.assertEqual(channel.messages, ["Ask failed (transport exit 7)\n\nboom"])

    async def test_resume_timeout_failure_includes_resume_button_delivery(self) -> None:
        fixture = DepsFixture(
            transport_result=(
                1,
                "ERROR: resident app-server transport failed: "
                + "Timed out waiting for app-server response to thread/resume.\n\nRun `!resume`.",
            )
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(channel.messages, [])
        self.assertEqual(len(fixture.resume_failures), 1)
        failure, target_thread_id = fixture.resume_failures[0]
        self.assertEqual(target_thread_id, "thread-1")
        self.assertIn("thread/resume", failure)

    async def test_wrong_thread_failure_hides_transport_details(self) -> None:
        output = (
            "Prompt landed in a different thread after app-server delivery. "
            "Expected 33income-hometax-form:019ecf32, "
            "but it was recorded in 33income-hometax-form:019ed92d."
        )
        fixture = DepsFixture(transport_result=(1, output))
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "019ecf32",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(fixture.deactivated, ["019ecf32"])
        self.assertEqual(
            channel.messages,
            [
                "Ask failed: Codex recorded this message in a different thread. I did not resend it here."
            ],
        )
        self.assertNotIn("019ecf32", channel.messages[0])
        self.assertNotIn("019ed92d", channel.messages[0])
        self.assertNotIn("Expected", channel.messages[0])

    async def test_busy_failure_uses_app_menu_without_failure_message(self) -> None:
        fixture = DepsFixture(
            transport_result=(1, "selected thread is still busy"),
            busy=True,
            app_menu_result=True,
        )
        channel = FakeChannel()

        result = await mapped_delivery.handle_mapped_prompt_delivery(
            channel,
            "please run",
            "thread-1",
            deps=fixture.build(),
        )

        self.assertTrue(result.handled)
        self.assertEqual(fixture.deactivated, ["thread-1"])
        self.assertEqual(
            fixture.app_menu_calls,
            [
                (
                    "thread-1",
                    "selected thread is still busy",
                    "ask_transport_no_wait_busy",
                )
            ],
        )
        self.assertEqual(channel.messages, [])


class DeliveryPendingPredicateTests(unittest.TestCase):
    def test_app_server_pending_output_is_delivery_pending(self) -> None:
        output = "\n".join(
            [
                "transport: resident-app-server turn/start",
                "[delivery_pending]",
                "Codex app-server accepted the request, but local session recording was not confirmed before the deadline.",
            ]
        )

        self.assertTrue(steering.is_ipc_delivery_confirmation_timeout(output))
