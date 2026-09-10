from __future__ import annotations

from dataclasses import dataclass, field
from types import TracebackType
import unittest

import codex_discord_prompt_mapped_delivery_factory as factory


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 100


@dataclass(frozen=True, slots=True)
class FakeTypingContext:
    async def __aenter__(self) -> None:
        return None

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None:
        _ = exc_type, exc, traceback
        return None


@dataclass(slots=True)  # noqa: MUTABLE_OK
class FactoryFixture:
    prepared_calls: list[tuple[FakeChannel, str | None]] = field(default_factory=list)
    selected_thread_ids: list[str] = field(default_factory=list)
    typing_calls: list[tuple[FakeChannel, str]] = field(default_factory=list)
    transport_calls: list[tuple[str, str | None]] = field(default_factory=list)
    chunk_calls: list[tuple[FakeChannel, str, str | None]] = field(default_factory=list)
    app_menu_calls: list[tuple[FakeChannel, str | None, str, str]] = field(default_factory=list)
    resume_failure_calls: list[tuple[FakeChannel, str, str]] = field(default_factory=list)
    marked_discord_origin_prompts: list[tuple[str | None, str]] = field(default_factory=list)
    deactivated: list[str | None] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)

    async def prepare(self, channel: FakeChannel, target_thread_id: str | None) -> bool:
        self.prepared_calls.append((channel, target_thread_id))
        return True

    def set_selected_thread_id(self, thread_id: str) -> None:
        self.selected_thread_ids.append(thread_id)

    def typing(self, channel: FakeChannel, *, context: str) -> FakeTypingContext:
        self.typing_calls.append((channel, context))
        return FakeTypingContext()

    def run_transport_sync(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        self.transport_calls.append((prompt, target_thread_id))
        return 23, "transport-output"

    async def send_chunks(
        self,
        channel: FakeChannel,
        content: str,
        *,
        context: str | None = None,
    ) -> None:
        self.chunk_calls.append((channel, content, context))

    def mark_recent_discord_origin_prompt(self, target_thread_id: str | None, prompt: str) -> None:
        self.marked_discord_origin_prompts.append((target_thread_id, prompt))

    def is_delivery_confirmation_timeout(self, output: str) -> bool:
        return output == "pending-output"

    def format_pending_ask_delivery_output(self, output: str) -> str:
        return f"pending:{output}"

    def is_selected_thread_busy_error(self, exit_code: int, output: str) -> bool:
        return exit_code == 77 and output == "busy-output"

    async def send_codex_app_menu_if_available(
        self,
        channel: FakeChannel,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> bool:
        self.app_menu_calls.append((channel, target_thread_id, output, reason))
        return True

    async def send_resume_failure(
        self,
        channel: FakeChannel,
        content: str,
        target_thread_id: str,
    ) -> None:
        self.resume_failure_calls.append((channel, content, target_thread_id))

    def format_log_text_len(self, text: str | None) -> int:
        return len(text or "")


class MappedPromptDeliveryFactoryTests(unittest.IsolatedAsyncioTestCase):
    async def test_factory_wires_supplied_dependencies_and_transport_adapter(self) -> None:
        fixture = FactoryFixture()
        channel = FakeChannel()
        prepare = fixture.prepare
        set_selected_thread_id = fixture.set_selected_thread_id
        typing = fixture.typing
        run_transport_sync = fixture.run_transport_sync
        send_chunks = fixture.send_chunks
        is_delivery_confirmation_timeout = fixture.is_delivery_confirmation_timeout
        format_pending_ask_delivery_output = fixture.format_pending_ask_delivery_output
        async def release_session_mirror_output_target(thread_id: str | None) -> bool:
            fixture.deactivated.append(thread_id)
            return True
        is_selected_thread_busy_error = fixture.is_selected_thread_busy_error
        send_codex_app_menu_if_available = fixture.send_codex_app_menu_if_available
        send_resume_failure = fixture.send_resume_failure
        format_log_text_len = fixture.format_log_text_len
        mark_recent_discord_origin_prompt = fixture.mark_recent_discord_origin_prompt
        log = fixture.logs.append

        deps = factory.make_mapped_prompt_delivery_deps(
            prepare_mapped_session_mirror_output=prepare,
            set_selected_thread_id=set_selected_thread_id,
            channel_typing=typing,
            run_transport_prompt_no_wait=run_transport_sync,
            send_chunks=send_chunks,
            is_delivery_confirmation_timeout=is_delivery_confirmation_timeout,
            format_pending_ask_delivery_output=format_pending_ask_delivery_output,
            release_session_mirror_output_target=release_session_mirror_output_target,
            is_selected_thread_busy_error=is_selected_thread_busy_error,
            send_codex_app_menu_if_available=send_codex_app_menu_if_available,
            send_resume_failure=send_resume_failure,
            format_log_text_len=format_log_text_len,
            mark_recent_discord_origin_prompt=mark_recent_discord_origin_prompt,
            log=log,
        )

        self.assertIs(deps.prepare_mapped_session_mirror_output, prepare)
        self.assertIs(deps.set_selected_thread_id, set_selected_thread_id)
        self.assertIs(deps.channel_typing, typing)
        self.assertIsNot(deps.run_transport_prompt_no_wait, run_transport_sync)
        self.assertIs(deps.send_chunks, send_chunks)
        self.assertIs(deps.mark_recent_discord_origin_prompt, mark_recent_discord_origin_prompt)
        self.assertIs(deps.is_delivery_confirmation_timeout, is_delivery_confirmation_timeout)
        self.assertIs(deps.format_pending_ask_delivery_output, format_pending_ask_delivery_output)
        self.assertIs(deps.release_session_mirror_output_target, release_session_mirror_output_target)
        self.assertIs(deps.is_selected_thread_busy_error, is_selected_thread_busy_error)
        self.assertIs(deps.send_codex_app_menu_if_available, send_codex_app_menu_if_available)
        self.assertIs(deps.send_resume_failure, send_resume_failure)
        self.assertIs(deps.format_log_text_len, format_log_text_len)
        self.assertIs(deps.log, log)

        self.assertTrue(await deps.prepare_mapped_session_mirror_output(channel, "thread-1"))
        deps.set_selected_thread_id("thread-1")
        _ = deps.channel_typing(channel, context="typing-context")
        deps.mark_recent_discord_origin_prompt("thread-1", "rewritten prompt")
        await deps.send_chunks(channel, "chunk-body", context="chunk-context")
        self.assertTrue(deps.is_delivery_confirmation_timeout("pending-output"))
        self.assertEqual(deps.format_pending_ask_delivery_output("output"), "pending:output")
        self.assertTrue(await deps.release_session_mirror_output_target("thread-1"))
        self.assertTrue(deps.is_selected_thread_busy_error(77, "busy-output"))
        self.assertTrue(
            await deps.send_codex_app_menu_if_available(
                channel,
                "thread-1",
                "busy-output",
                reason="busy-reason",
            )
        )
        await deps.send_resume_failure(channel, "resume failed", "thread-1")
        self.assertEqual(deps.format_log_text_len("abc"), 3)
        deps.log("logged")

        self.assertEqual(await deps.run_transport_prompt_no_wait("prompt text", "thread-1"), (23, "transport-output"))
        self.assertEqual(fixture.transport_calls, [("prompt text", "thread-1")])
        self.assertEqual(fixture.prepared_calls, [(channel, "thread-1")])
        self.assertEqual(fixture.selected_thread_ids, ["thread-1"])
        self.assertEqual(fixture.typing_calls, [(channel, "typing-context")])
        self.assertEqual(fixture.marked_discord_origin_prompts, [("thread-1", "rewritten prompt")])
        self.assertEqual(fixture.chunk_calls, [(channel, "chunk-body", "chunk-context")])
        self.assertEqual(fixture.deactivated, ["thread-1"])
        self.assertEqual(fixture.app_menu_calls, [(channel, "thread-1", "busy-output", "busy-reason")])
        self.assertEqual(fixture.resume_failure_calls, [(channel, "resume failed", "thread-1")])
        self.assertEqual(fixture.logs, ["logged"])
