from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias

from codex_desktop_bridge_final_answer_types import WatchForFinalAnswerResult as WatchResult
from codex_desktop_bridge_sidecar_thread import StartTurnResult
from codex_thread_models import ThreadInfo, WindowInfo

SessionOffsets: TypeAlias = dict[str, tuple[ThreadInfo, Path, int]]
IpcTurnResult: TypeAlias = dict[str, str]


class GetThreadBusyState(Protocol):
    def __call__(self, thread: ThreadInfo, *, allow_resume: bool = True) -> str: ...


class SnapshotRecentSessionOffsets(Protocol):
    def __call__(
        self,
        *,
        limit: int = 10,
        include_threads: list[ThreadInfo] | None = None,
    ) -> SessionOffsets: ...


class WaitForPromptDelivery(Protocol):
    def __call__(
        self,
        session_offsets: SessionOffsets,
        prompt: str,
        timeout_sec: float = 4.0,
    ) -> ThreadInfo | None: ...


class StartTurnViaSidecar(Protocol):
    def __call__(
        self,
        thread: ThreadInfo,
        prompt: str,
        *,
        timeout_sec: float = 10.0,
        keep_client_open: bool = False,
    ) -> StartTurnResult: ...


class StartTurnViaIpc(Protocol):
    def __call__(
        self,
        thread: ThreadInfo,
        prompt: str,
        timeout_sec: float = 10.0,
        *,
        allow_ui_recovery: bool = False,
    ) -> IpcTurnResult: ...


class SendPromptToCodex(Protocol):
    def __call__(
        self,
        *,
        prompt: str,
        click_x_ratio: float,
        click_y_offset: int,
        skip_click: bool,
    ) -> WindowInfo: ...


class StartBackgroundWatch(Protocol):
    def __call__(
        self,
        *,
        thread: ThreadInfo,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_output: bool,
    ) -> bool: ...


class WatchForFinalAnswer(Protocol):
    def __call__(
        self,
        *,
        session_path: Path,
        start_offset: int,
        timeout_sec: float,
        include_commentary: bool,
        stream_live: bool,
    ) -> WatchResult: ...


@dataclass(frozen=True, slots=True)
class CommandAskDeps:
    choose_thread: Callable[[str | None, str | None], ThreadInfo]
    format_title_preview: Callable[[str], str]
    get_thread_ui_name: Callable[[str, ThreadInfo | None], str | None]
    get_thread_busy_state: GetThreadBusyState
    describe_thread_busy_state: Callable[[str], str]
    snapshot_recent_session_offsets: SnapshotRecentSessionOffsets
    wait_for_prompt_delivery: WaitForPromptDelivery
    start_turn_via_sidecar: StartTurnViaSidecar
    start_turn_via_ipc: StartTurnViaIpc
    activate_thread_in_ui: Callable[[ThreadInfo], str]
    verify_thread_in_ui: Callable[[ThreadInfo], str | None]
    send_prompt_to_codex: SendPromptToCodex
    start_background_watch: StartBackgroundWatch
    watch_for_final_answer: WatchForFinalAnswer
    get_thread_label: Callable[[ThreadInfo], str]
    make_console_safe_text: Callable[[str], str]
