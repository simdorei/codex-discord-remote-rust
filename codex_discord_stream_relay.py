from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
import traceback
from collections.abc import Awaitable
from concurrent.futures import Future

from codex_discord_stream_relay_lines import classify_stream_relay_line
from codex_discord_stream_relay_types import (
    FormatLogTextLenFunc,
    HadSteeringHandoffSinceFunc,
    InteractiveNoticeOptions as InteractiveNoticeOptions,
    IsDiscordRelayStaleFunc,
    LogFunc,
    ParseInteractiveNoticeFunc,
    RegisterDiscordRelayFunc,
    RelayChannel as RelayChannel,
    RelayMode,
    SendChunksFunc,
    SendInteractivePromptFunc,
)


class DiscordAskRelay:
    def __init__(
        self,
        loop: asyncio.AbstractEventLoop,
        channel: RelayChannel,
        target_thread_id: str | None,
        target_ref: str,
        *,
        quiet_notice_delay_sec: float,
        suppress_after_steering_since: float | None = None,
        send_timeout_blocks: bool = True,
        send_commentary_blocks: bool = False,
        send_final_blocks: bool = True,
        send_chunks_func: SendChunksFunc,
        parse_interactive_notice_func: ParseInteractiveNoticeFunc,
        send_interactive_prompt_func: SendInteractivePromptFunc,
        register_discord_relay_func: RegisterDiscordRelayFunc,
        is_discord_relay_stale_func: IsDiscordRelayStaleFunc,
        had_steering_handoff_since_func: HadSteeringHandoffSinceFunc,
        log_func: LogFunc,
        format_log_text_len_func: FormatLogTextLenFunc,
    ) -> None:
        self.loop: asyncio.AbstractEventLoop = loop
        self.channel: RelayChannel = channel
        self.target_thread_id: str | None = target_thread_id
        self.target_ref: str = target_ref
        self.quiet_notice_delay_sec: float = quiet_notice_delay_sec
        self.suppress_after_steering_since: float | None = suppress_after_steering_since
        self.send_timeout_blocks: bool = send_timeout_blocks
        self.send_commentary_blocks: bool = send_commentary_blocks
        self.send_final_blocks: bool = send_final_blocks
        self._send_chunks: SendChunksFunc = send_chunks_func
        self._parse_interactive_notice: ParseInteractiveNoticeFunc = parse_interactive_notice_func
        self._send_interactive_prompt: SendInteractivePromptFunc = send_interactive_prompt_func
        self._is_discord_relay_stale: IsDiscordRelayStaleFunc = is_discord_relay_stale_func
        self._had_steering_handoff_since: HadSteeringHandoffSinceFunc = had_steering_handoff_since_func
        self._log: LogFunc = log_func
        self._format_log_text_len: FormatLogTextLenFunc = format_log_text_len_func
        self.relay_generation: int = register_discord_relay_func(target_thread_id)
        self.mode: RelayMode | None = None
        self.block_lines: list[str] = []
        self.sent_live: bool = False
        self.quiet_notice_sent: bool = False
        self.suppressed_after_steering: bool = False
        self.saw_final: bool = False
        self.saw_aborted: bool = False
        self.saw_failed: bool = False
        self.saw_timeout: bool = False
        self._send_futures: list[Future[None]] = []
        self._quiet_notice_future: Future[None] | None = None

    def _schedule_send(self, send_coro: Awaitable[None]) -> None:
        self._cancel_quiet_notice()
        previous = self._send_futures[-1] if self._send_futures else None

        async def send_in_order() -> None:
            if previous is not None:
                try:
                    await asyncio.wrap_future(previous)
                except Exception:  # noqa: BROAD_EXCEPT_OK
                    self._log("discord_relay_previous_send_failed\n" + traceback.format_exc())
            await send_coro

        future = asyncio.run_coroutine_threadsafe(send_in_order(), self.loop)
        self._send_futures.append(future)

    def _send(self, text: str) -> None:
        self._schedule_send(self._send_chunks(self.channel, text))

    async def _send_quiet_notice_after_delay(self) -> None:
        await asyncio.sleep(max(0.0, self.quiet_notice_delay_sec))
        if (
            self.sent_live
            or self.quiet_notice_sent
            or self.saw_final
            or self.saw_aborted
            or self.saw_failed
            or self.saw_timeout
        ):
            return
        await self._send_chunks(
            self.channel,
            "\n".join(
                [
                    "Codex is still working.",
                    "",
                    "High-context threads can stay quiet while Codex compacts context before the next visible reply.",
                ]
            ),
        )
        self.quiet_notice_sent = True

    def _schedule_quiet_notice(self) -> None:
        if self.quiet_notice_delay_sec < 0:
            return
        current = self._quiet_notice_future
        if current is not None and not current.done():
            return
        self._quiet_notice_future = asyncio.run_coroutine_threadsafe(
            self._send_quiet_notice_after_delay(),
            self.loop,
        )

    def _cancel_quiet_notice(self) -> None:
        future = self._quiet_notice_future
        if future is not None and not future.done():
            _ = future.cancel()

    def _should_suppress_for_steering(self) -> bool:
        if self._is_discord_relay_stale(self.target_thread_id, self.relay_generation):
            return True
        if self.suppress_after_steering_since is None:
            return False
        if self.mode != "final":
            return False
        return self._had_steering_handoff_since(
            self.target_thread_id,
            self.suppress_after_steering_since,
        )

    def _send_interactive_notice_if_detected(self, text: str) -> bool:
        state, prompt, options = self._parse_interactive_notice(text)
        if not state or not self.target_thread_id:
            return False
        self._schedule_send(
            self._send_interactive_prompt(
                self.channel,
                self.target_thread_id,
                self.target_ref,
                state,
                prompt,
                options,
            )
        )
        self.sent_live = True
        return True

    def _send_block(self) -> None:
        text = "\n".join(self.block_lines).strip()
        if not text:
            self.block_lines = []
            return
        if self._should_suppress_for_steering():
            self.suppressed_after_steering = True
            self._log(
                f"discord_relay_suppressed_after_steering target={self.target_thread_id or '-'} "
                + f"mode={self.mode or '-'} text_len={self._format_log_text_len(text)}"
            )
            self.block_lines = []
            return
        match self.mode:  # noqa: MATCH_OK
            case "commentary":
                if not self._send_interactive_notice_if_detected(text):
                    if self.send_commentary_blocks:
                        self._send(f"In progress\n\n{text}")
                        self.sent_live = True
            case "final":
                if not self._send_interactive_notice_if_detected(text):
                    self.saw_final = True
                    if self.send_final_blocks:
                        self._send(f"Final\n\n{text}")
                        self.sent_live = True
            case "timeout":
                self.saw_timeout = True
                if self.send_timeout_blocks:
                    self._send(f"Timed out\n\n{text}")
                    self.sent_live = True
            case "failed":
                self.saw_failed = True
                self._send(f"Failed\n\n{text}")
                self.sent_live = True
            case "transport_error":
                self.saw_failed = True
                self._send(f"Transport error\n\n{text}")
                self.sent_live = True
            case None:
                pass
        self.block_lines = []

    def feed_line(self, line: str) -> None:
        line_kind = classify_stream_relay_line(line)
        if line_kind == "commentary":
            self._send_block()
            self.mode = "commentary"
            return
        if line_kind == "final":
            self._send_block()
            self.mode = "final"
            return
        if line_kind == "timeout":
            self._send_block()
            self.mode = "timeout"
            self.saw_timeout = True
            return
        if line_kind in {"failed", "transport_error"}:
            self._send_block()
            self.mode = "failed" if line_kind == "failed" else "transport_error"
            self.saw_failed = True
            return
        if line_kind == "aborted":
            self._send_block()
            self.mode = None
            self.saw_aborted = True
            self._send("Aborted.")
            self.sent_live = True
            return
        if line_kind == "ready":
            self._send_block()
            self.mode = None
            return
        if line_kind == "waiting":
            if line.startswith("[waiting_for_final_answer]"):
                self._schedule_quiet_notice()
            return

        if self.mode in {"commentary", "failed", "final", "timeout", "transport_error"}:
            self.block_lines.append(line)
            return

        if line_kind == "ignored":
            return

    def finish(self) -> None:
        self._cancel_quiet_notice()
        self._send_block()
        quiet_future = self._quiet_notice_future
        if quiet_future is not None and quiet_future.done():
            if not quiet_future.cancelled():
                try:
                    quiet_future.result(timeout=0)
                except Exception:  # noqa: BROAD_EXCEPT_OK
                    self._log("discord_relay_quiet_notice_failed\n" + traceback.format_exc())
        for future in self._send_futures:
            try:
                future.result(timeout=30)
            except Exception:  # noqa: BROAD_EXCEPT_OK
                self._log("discord_relay_send_failed\n" + traceback.format_exc())
