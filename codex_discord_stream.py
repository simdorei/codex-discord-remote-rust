"""Stream relay helpers for Discord ask/watch output."""

from __future__ import annotations

from collections.abc import Callable
from contextlib import AbstractContextManager
from pathlib import Path
from typing import Literal, NotRequired, Protocol, TypedDict

from codex_discord_steering import NativeExactWatchTarget, SteeringPromptResult
from codex_discord_stream_relay import DiscordAskRelay as DiscordAskRelay

LineStreamFunc = Callable[[str], None]
RunBridgeCommandStreamFunc = Callable[[list[str], LineStreamFunc], tuple[int, str]]
ShouldRetryAskWithUiFunc = Callable[[int, str], bool]


class AskStreamRelay(Protocol):
    def feed_line(self, line: str) -> None: ...
    def finish(self) -> None: ...


class BuildUiAskArgvFunc(Protocol):
    def __call__(
        self,
        prompt: str,
        *,
        target_thread_id: str | None,
        force_while_busy: bool,
        wait: bool,
    ) -> list[str]: ...


class WatchForFinalAnswerResult(TypedDict):
    status: Literal["aborted", "failed", "final", "progress", "timeout", "transport_error"]
    commentary: list[str]
    final_answer: str
    streamed_live: bool
    final_streamed_live: bool
    error_message: NotRequired[str]
    interrupt_origin: NotRequired[str]


class WatchForFinalAnswerFunc(Protocol):
    def __call__(
        self,
        *,
        session_path: Path,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_live: bool = False,
        stream_label: str = "",
        stream_callback: LineStreamFunc | None = None,
        expected_turn_id: str | None = None,
    ) -> WatchForFinalAnswerResult: ...


def build_stream_ask_argv(
    prompt: str,
    *,
    force_while_busy: bool = False,
    wait: bool = True,
    target_thread_id: str | None = None,
    use_sidecar: bool = False,
    ipc_recover_ui: bool = False,
    no_fallback: bool = False,
) -> list[str]:
    argv = [
        "ask",
        "--sidecar" if use_sidecar else "--ipc",
        "--foreground",
        "--stream",
        "--include-commentary",
        "--timeout",
        "0",
    ]
    if not use_sidecar and ipc_recover_ui:
        argv.insert(2, "--ipc-recover-ui")
    if not use_sidecar and no_fallback:
        argv.append("--no-fallback")
    if target_thread_id:
        argv.extend(["--thread-id", target_thread_id])
    if force_while_busy:
        argv.append("--force-while-busy")
    if not wait:
        argv.append("--no-wait")
    argv.append(prompt)
    return argv


def ensure_ui_stream_flags(ui_argv: list[str]) -> list[str]:
    if "--stream" in ui_argv:
        return ui_argv
    result = list(ui_argv)
    result.insert(result.index("--timeout"), "--include-commentary")
    result.insert(result.index("--include-commentary"), "--stream")
    return result


def run_ask_stream(
    prompt: str,
    relay: AskStreamRelay,
    *,
    force_while_busy: bool = False,
    wait: bool = True,
    target_thread_id: str | None = None,
    use_sidecar: bool = False,
    ipc_recover_ui: bool = False,
    no_fallback: bool = False,
    allow_ui_fallback: bool = False,
    run_bridge_command_stream_func: RunBridgeCommandStreamFunc,
    should_retry_ask_with_ui_func: ShouldRetryAskWithUiFunc,
    build_ui_ask_argv_func: BuildUiAskArgvFunc,
    ui_fallback_lock: AbstractContextManager[bool],
) -> tuple[int, str]:
    argv = build_stream_ask_argv(
        prompt,
        force_while_busy=force_while_busy,
        wait=wait,
        target_thread_id=target_thread_id,
        use_sidecar=use_sidecar,
        ipc_recover_ui=ipc_recover_ui,
        no_fallback=no_fallback,
    )
    exit_code, output = run_bridge_command_stream_func(argv, relay.feed_line)
    if allow_ui_fallback and not use_sidecar and should_retry_ask_with_ui_func(exit_code, output):
        relay.feed_line("[commentary]")
        relay.feed_line("IPC attach failed for this Codex thread. Retrying through the Codex UI.")
        relay.feed_line("[ready]")
        ui_argv = build_ui_ask_argv_func(
            prompt,
            target_thread_id=target_thread_id,
            force_while_busy=True,
            wait=wait,
        )
        ui_argv = ensure_ui_stream_flags(ui_argv)
        with ui_fallback_lock:
            exit_code, output = run_bridge_command_stream_func(ui_argv, relay.feed_line)
    relay.finish()
    return exit_code, output


def run_steering_watch_stream(
    steering_result: SteeringPromptResult,
    relay: AskStreamRelay,
    *,
    timeout_sec: float = 0,
    watch_for_final_answer_func: WatchForFinalAnswerFunc,
) -> tuple[int, str]:
    session_path: str | None = steering_result.session_path
    start_offset: int | None = steering_result.start_offset
    if not session_path or start_offset is None:
        relay.finish()
        return 0, ""

    relay.feed_line("[waiting_for_final_answer]")
    relay.feed_line("Use Ctrl+C to stop waiting after the prompt is sent.")
    output_lines = [
        "[waiting_for_final_answer]",
        "Use Ctrl+C to stop waiting after the prompt is sent.",
    ]

    def relay_stream_line(line: str) -> None:
        relay.feed_line(line)
        output_lines.append(line)

    try:
        if isinstance(steering_result.watch_target, NativeExactWatchTarget):
            result = watch_for_final_answer_func(
                session_path=Path(session_path),
                start_offset=start_offset,
                timeout_sec=timeout_sec,
                include_commentary=True,
                stream_live=True,
                stream_callback=relay_stream_line,
                expected_turn_id=steering_result.watch_target.turn_id,
            )
        else:
            result = watch_for_final_answer_func(
                session_path=Path(session_path),
                start_offset=start_offset,
                timeout_sec=timeout_sec,
                include_commentary=True,
                stream_live=True,
                stream_callback=relay_stream_line,
            )

        if result["final_answer"]:
            if result.get("final_streamed_live"):
                relay.feed_line("[ready]")
                output_lines.append("[ready]")
            else:
                final_lines = str(result["final_answer"]).splitlines()
                for line in ["[final_answer]", *final_lines, "", "[ready]"]:
                    relay.feed_line(line)
                    output_lines.append(line)
            return 0, "\n".join(output_lines).strip()

        if result["status"] == "aborted":
            relay.feed_line("[aborted]")
            output_lines.append("[aborted]")
            return 0, "\n".join(output_lines).strip()

        if result["status"] == "progress":
            relay.feed_line("[ready]")
            output_lines.append("[ready]")
            return 0, "\n".join(output_lines).strip()

        if result["status"] in {"failed", "transport_error"}:
            marker = "[failed]" if result["status"] == "failed" else "[transport_error]"
            relay.feed_line(marker)
            output_lines.append(marker)
            error_message = result.get("error_message") or "Codex terminal state could not be verified."
            for line in error_message.splitlines():
                relay.feed_line(line)
                output_lines.append(line)
            relay.feed_line("[ready]")
            output_lines.append("[ready]")
            return 1, "\n".join(output_lines).strip()

        relay.feed_line("[timeout]")
        output_lines.append("[timeout]")
        commentary = result.get("commentary") or []
        if commentary and not result.get("streamed_live"):
            for line in str(commentary[-1]).splitlines():
                relay.feed_line(line)
                output_lines.append(line)
        return 2, "\n".join(output_lines).strip()
    finally:
        relay.finish()
