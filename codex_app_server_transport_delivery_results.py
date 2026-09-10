from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import codex_desktop_bridge as bridge
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class AppServerDeliveryResult:
    exit_code: int
    output: str
    thread_id: str | None = None
    turn_id: str | None = None
    target_ref: str = ""
    session_path: str | None = None
    start_offset: int | None = None
    delivery_pending: bool = False
    transport: str = "resident-app-server"


@dataclass(frozen=True, slots=True)
class DeliveryContext:
    thread: ThreadInfo
    target_ref: str
    recent_offsets: dict[str, tuple[ThreadInfo, Path, int]]
    session_path: Path
    start_offset: int | None


class BridgeModule(Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...
    def get_thread_workspace_ref(self, thread: ThreadInfo, threads: list[ThreadInfo] | None = None) -> str: ...
    def snapshot_recent_session_offsets(
        self,
        limit: int = 10,
        include_threads: list[ThreadInfo] | None = None,
    ) -> dict[str, tuple[ThreadInfo, Path, int]]: ...
    def wait_for_prompt_delivery(
        self,
        session_offsets: dict[str, tuple[ThreadInfo, Path, int]],
        prompt: str,
        timeout_sec: float = 4.0,
    ) -> ThreadInfo | None: ...
    def get_thread_label(self, thread: ThreadInfo) -> str: ...
    def format_title_preview(self, value: str, limit: int = 120) -> str: ...


def build_delivery_context(
    target_thread_id: str | None,
    *,
    bridge_module: BridgeModule = bridge,
) -> DeliveryContext:
    thread = bridge_module.choose_thread(target_thread_id, None)
    target_ref = bridge_module.get_thread_workspace_ref(thread)
    recent_offsets = bridge_module.snapshot_recent_session_offsets(
        limit=10,
        include_threads=[thread],
    )
    session_path = Path(thread.rollout_path)
    start_offset = session_path.stat().st_size if session_path.exists() else None
    return DeliveryContext(thread, target_ref, recent_offsets, session_path, start_offset)


def wait_for_delivery(
    context: DeliveryContext,
    prompt: str,
    *,
    bridge_module: BridgeModule = bridge,
    timeout_sec: float,
) -> ThreadInfo | None:
    if timeout_sec <= 0:
        return None
    return bridge_module.wait_for_prompt_delivery(
        context.recent_offsets,
        prompt,
        timeout_sec=timeout_sec,
    )


def cross_thread_delivery_result(
    context: DeliveryContext,
    *,
    delivered_thread: ThreadInfo,
    turn_id: str | None,
    bridge_module: BridgeModule = bridge,
) -> AppServerDeliveryResult:
    return AppServerDeliveryResult(
        1,
        "Prompt landed in a different thread after app-server delivery. "
        + f"Expected {bridge_module.get_thread_label(context.thread)}, "
        + f"but it was recorded in {bridge_module.get_thread_label(delivered_thread)}.",
        thread_id=context.thread.id,
        turn_id=turn_id,
        target_ref=context.target_ref,
        session_path=str(context.session_path),
        start_offset=context.start_offset,
    )


def successful_delivery_result(
    context: DeliveryContext,
    *,
    method: str,
    turn_id: str | None,
    delivered_thread: ThreadInfo | None,
    delivery_pending: bool,
    bridge_module: BridgeModule = bridge,
) -> AppServerDeliveryResult:
    return AppServerDeliveryResult(
        0,
        _make_output(
            context=context,
            method=method,
            turn_id=turn_id,
            delivered_thread=delivered_thread,
            delivery_pending=delivery_pending,
            bridge_module=bridge_module,
        ),
        thread_id=context.thread.id,
        turn_id=turn_id,
        target_ref=context.target_ref,
        session_path=str(context.session_path),
        start_offset=context.start_offset,
        delivery_pending=delivery_pending,
    )


def _make_output(
    *,
    context: DeliveryContext,
    method: str,
    turn_id: str | None,
    delivered_thread: ThreadInfo | None,
    delivery_pending: bool,
    bridge_module: BridgeModule = bridge,
) -> str:
    thread = context.thread
    lines = [
        f"target_thread: {thread.id}",
        f"title: {bridge_module.format_title_preview(thread.title)}",
        f"cwd: {thread.cwd}",
        f"transport: resident-app-server {method}",
    ]
    if turn_id:
        lines.append(f"[app_server_delivery] turn_id={turn_id}")
    if delivered_thread is not None:
        lines.append(f"[delivery_verified] {bridge_module.get_thread_label(delivered_thread)}")
    elif delivery_pending:
        lines.append("[delivery_pending]")
        lines.append(
            "Codex app-server accepted the request, but local session recording was not confirmed before the deadline."
        )
        lines.append("Discord will keep watching the mapped session for the next Codex reply.")
    if context.target_ref:
        lines.append(f"thread_ref: {context.target_ref}")
    return "\n".join(lines)
