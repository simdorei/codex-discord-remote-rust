# pyright: reportAny=false, reportAttributeAccessIssue=false, reportImplicitOverride=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import io
from contextlib import redirect_stdout
from pathlib import Path
from typing import override

import codex_desktop_bridge_command_ask as command_ask
import codex_desktop_bridge_command_ask_types as ask_types
from codex_desktop_bridge_sidecar import CodexAppServerSidecar
from codex_thread_models import ThreadInfo, WindowInfo


class FakeSidecarClient(CodexAppServerSidecar):
    closed: bool = False

    @override
    def close(self) -> None:
        self.closed = True


class FakeDeps:
    def __init__(
        self,
        *,
        thread: ThreadInfo,
        busy_state: str = "idle",
        delivery_thread: ThreadInfo | None = None,
        sidecar_result: ask_types.StartTurnResult | None = None,
        ipc_result: ask_types.IpcTurnResult | None = None,
        watch_result: ask_types.WatchResult | None = None,
        watch_interrupt: bool = False,
        background_started: bool = True,
        window: WindowInfo | None = None,
        delivery_missing: bool = False,
    ) -> None:
        self.thread: ThreadInfo = thread
        self.busy_state: str = busy_state
        self.delivery_thread: ThreadInfo | None = None if delivery_missing else delivery_thread or thread
        self.sidecar_result: ask_types.StartTurnResult = sidecar_result or {
            "owner_client_id": "",
            "turn_id": "turn-sidecar",
            "attempts": "1",
        }
        self.ipc_result: ask_types.IpcTurnResult = ipc_result or {
            "owner_client_id": "client-1",
            "turn_id": "turn-ipc",
        }
        self.watch_result: ask_types.WatchResult = watch_result or final_result("done")
        self.watch_interrupt: bool = watch_interrupt
        self.background_started: bool = background_started
        self.window: WindowInfo = window or window_info()
        self.calls: list[str] = []
        self.delivery_timeouts: list[float] = []

    def as_command_deps(self) -> ask_types.CommandAskDeps:
        return ask_types.CommandAskDeps(
            choose_thread=self.choose_thread,
            format_title_preview=self.format_title_preview,
            get_thread_ui_name=self.get_thread_ui_name,
            get_thread_busy_state=self.get_thread_busy_state,
            describe_thread_busy_state=self.describe_thread_busy_state,
            snapshot_recent_session_offsets=self.snapshot_recent_session_offsets,
            wait_for_prompt_delivery=self.wait_for_prompt_delivery,
            start_turn_via_sidecar=self.start_turn_via_sidecar,
            start_turn_via_ipc=self.start_turn_via_ipc,
            activate_thread_in_ui=self.activate_thread_in_ui,
            verify_thread_in_ui=self.verify_thread_in_ui,
            send_prompt_to_codex=self.send_prompt_to_codex,
            start_background_watch=self.start_background_watch,
            watch_for_final_answer=self.watch_for_final_answer,
            get_thread_label=self.get_thread_label,
            make_console_safe_text=self.make_console_safe_text,
        )

    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        self.calls.append(f"choose:{thread_id}:{cwd}")
        return self.thread

    def format_title_preview(self, title: str) -> str:
        return title

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None:
        _ = (thread_id, thread)
        return "Thread UI"

    def get_thread_busy_state(self, thread: ThreadInfo, *, allow_resume: bool = True) -> str:
        _ = (thread, allow_resume)
        return self.busy_state

    def describe_thread_busy_state(self, state: str) -> str:
        return f"{state} detail"

    def snapshot_recent_session_offsets(
        self,
        *,
        limit: int = 10,
        include_threads: list[ThreadInfo] | None = None,
    ) -> ask_types.SessionOffsets:
        _ = (limit, include_threads)
        return {}

    def wait_for_prompt_delivery(
        self,
        session_offsets: ask_types.SessionOffsets,
        prompt: str,
        timeout_sec: float = 4.0,
    ) -> ThreadInfo | None:
        _ = (session_offsets, prompt)
        self.delivery_timeouts.append(timeout_sec)
        return self.delivery_thread

    def start_turn_via_sidecar(
        self,
        thread: ThreadInfo,
        prompt: str,
        *,
        timeout_sec: float = 10.0,
        keep_client_open: bool = False,
    ) -> ask_types.StartTurnResult:
        _ = (thread, prompt, timeout_sec, keep_client_open)
        return dict(self.sidecar_result)

    def start_turn_via_ipc(
        self,
        thread: ThreadInfo,
        prompt: str,
        timeout_sec: float = 10.0,
        *,
        allow_ui_recovery: bool = False,
    ) -> ask_types.IpcTurnResult:
        _ = (thread, prompt, timeout_sec, allow_ui_recovery)
        return dict(self.ipc_result)

    def activate_thread_in_ui(self, thread: ThreadInfo) -> str:
        _ = thread
        return "sidebar:Thread [header]"

    def verify_thread_in_ui(self, thread: ThreadInfo) -> str | None:
        _ = thread
        return "header"

    def send_prompt_to_codex(
        self,
        *,
        prompt: str,
        click_x_ratio: float,
        click_y_offset: int,
        skip_click: bool,
    ) -> WindowInfo:
        _ = (prompt, click_x_ratio, click_y_offset, skip_click)
        return self.window

    def start_background_watch(
        self,
        *,
        thread: ThreadInfo,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_output: bool,
    ) -> bool:
        _ = (thread, start_offset, timeout_sec, include_commentary, stream_output)
        return self.background_started

    def watch_for_final_answer(
        self,
        *,
        session_path: Path,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_live: bool,
    ) -> ask_types.WatchResult:
        _ = (session_path, start_offset, timeout_sec, include_commentary, stream_live)
        if self.watch_interrupt:
            raise KeyboardInterrupt
        return self.watch_result

    def get_thread_label(self, thread: ThreadInfo) -> str:
        return thread.id

    def make_console_safe_text(self, value: str) -> str:
        return value


def fake_sidecar_client() -> FakeSidecarClient:
    client = FakeSidecarClient.__new__(FakeSidecarClient)
    client.closed = False
    return client


def run_with_output(args: argparse.Namespace, deps: FakeDeps) -> tuple[str, int]:
    stdout = io.StringIO()
    with redirect_stdout(stdout):
        exit_code = command_ask.run_command_ask(args, deps=deps.as_command_deps())
    return stdout.getvalue(), exit_code


def args(
    *,
    prompt: str = "prompt",
    thread_id: str = "thread-1",
    cwd: str | None = None,
    dry_run: bool = False,
    force_while_busy: bool = False,
    ipc: bool = True,
    sidecar: bool = False,
    ipc_recover_ui: bool = False,
    background: bool = False,
    wait: bool = True,
    timeout: float = 30.0,
    include_commentary: bool = False,
    stream: bool = False,
    switch_thread: bool = False,
    click: bool = False,
    click_x_ratio: float = 0.5,
    click_y_offset: int = 90,
) -> argparse.Namespace:
    return argparse.Namespace(
        thread_id=thread_id,
        cwd=cwd,
        prompt=prompt,
        dry_run=dry_run,
        force_while_busy=force_while_busy,
        ipc=ipc,
        sidecar=sidecar,
        ipc_recover_ui=ipc_recover_ui,
        background=background,
        wait=wait,
        timeout=timeout,
        include_commentary=include_commentary,
        stream=stream,
        switch_thread=switch_thread,
        click=click,
        click_x_ratio=click_x_ratio,
        click_y_offset=click_y_offset,
    )


def thread(root: Path, thread_id: str) -> ThreadInfo:
    session_path = root / f"{thread_id}.jsonl"
    session_path.write_text("old", encoding="utf-8")
    return ThreadInfo(
        id=thread_id,
        title=f"Title {thread_id}",
        cwd=str(root),
        updated_at=1,
        rollout_path=str(session_path),
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


def missing_thread(root: Path) -> ThreadInfo:
    return ThreadInfo(
        id="missing",
        title="Missing",
        cwd=str(root),
        updated_at=1,
        rollout_path=str(root / "missing.jsonl"),
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


def window_info() -> WindowInfo:
    return WindowInfo(hwnd=100, title="Codex", left=1, top=2, right=3, bottom=4)


def final_result(text: str) -> ask_types.WatchResult:
    return {
        "status": "final",
        "commentary": [],
        "final_answer": text,
        "streamed_live": False,
        "final_streamed_live": False,
    }
