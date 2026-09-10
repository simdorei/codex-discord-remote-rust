from __future__ import annotations

from collections.abc import Mapping
from typing import cast

import codex_discord_session_mirror_item_builders as item_builders
from codex_discord_session_mirror_item_append import (
    CollectionContext,
    SessionPayload,
    append_agent_if_new,
    append_item,
    append_user_if_new,
    remember,
)
from codex_discord_session_mirror_item_builders import SessionEvent, SessionMirrorItem
from codex_app_server_transport_goal import is_terminal_goal_status
from codex_app_server_transport_turn_outcomes import InterruptOrigin, TurnCompletion, TurnStatus

ABORTED_PAYLOAD_TYPES = {"turn_aborted", "task_aborted", "task_cancelled"}


def payload_turn_id(payload: SessionPayload) -> str:
    return str(payload.get("turn_id") or "").strip()


def is_final_goal_turn(ctx: CollectionContext, turn_id: str) -> bool:
    goal_update = ctx.goal_updates.get(turn_id)
    if goal_update is not None:
        return is_terminal_goal_status(goal_update.status)
    if ctx.goal_status is None:
        return True
    if not is_terminal_goal_status(ctx.goal_status):
        return False
    return bool(turn_id and turn_id == ctx.latest_terminal_turn_id)


def mark_terminal_turn(ctx: CollectionContext, turn_id: str) -> bool:
    if not turn_id:
        return True
    if turn_id in ctx.terminal_turn_ids:
        return False
    ctx.terminal_turn_ids.add(turn_id)
    return True


def event_payload(event: SessionEvent) -> SessionPayload | None:
    payload = event.get("payload")
    if payload is None:
        payload = {}
    if not isinstance(payload, Mapping):
        return None
    return cast(SessionPayload, payload)


def format_no_visible_reply_text(payload: SessionPayload) -> str:
    details: list[str] = []
    turn_id = str(payload.get("turn_id") or "").strip()
    if turn_id:
        details.append(f"turn_id={turn_id}")
    duration_ms = payload.get("duration_ms")
    if duration_ms is not None:
        details.append(f"duration_ms={duration_ms}")
    if not details:
        return "Codex turn completed without a visible reply."
    return f"Codex turn completed without a visible reply.\nDetails: {', '.join(details)}"


def append_no_visible_reply_if_needed(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    payload: SessionPayload,
) -> None:
    turn_id = payload_turn_id(payload)
    is_final = is_final_goal_turn(ctx, turn_id)
    append_item(
        ctx,
        items,
        event,
        kind="final" if is_final else "commentary",
        role="assistant",
        phase="no_visible_reply" if is_final else "goal_turn_complete",
        text=format_no_visible_reply_text(payload),
    )


def collect_agent_event_message(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    payload: SessionPayload,
) -> None:
    phase = str(payload.get("phase") or "commentary")
    text = str(payload.get("message") or "").strip()
    if not text:
        return
    if phase == "final_answer":
        return
    remember(ctx, ctx.seen_agent_messages, text)
    append_item(ctx, items, event, kind="commentary", role="assistant", phase=phase, text=text)


def collect_event_message(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    payload: SessionPayload,
) -> None:
    payload_type = str(payload.get("type") or "")
    if payload_type == "agent_message":
        collect_agent_event_message(ctx, items, event, payload)
        return
    if payload_type == "user_message":
        text = str(payload.get("message") or "").strip()
        if text:
            append_user_if_new(ctx, items, event, text, "input")
        return
    if payload_type not in ABORTED_PAYLOAD_TYPES | {"task_complete"}:
        return

    turn_id = payload_turn_id(payload)
    if not mark_terminal_turn(ctx, turn_id):
        return
    completion = ctx.turn_completions.get(turn_id)
    if completion is not None and _append_native_noncompletion(ctx, items, event, completion):
        return
    if payload_type == "task_complete" or (
        completion is not None and completion.status is TurnStatus.COMPLETED
    ):
        _append_completed_turn(ctx, items, event, payload, turn_id)
        return
    append_item(
        ctx,
        items,
        event,
        kind="aborted",
        role="assistant",
        phase=payload_type,
        text=item_builders.format_aborted_event_text(dict(payload)),
    )


def _append_native_noncompletion(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    completion: TurnCompletion,
) -> bool:
    if completion.status is TurnStatus.INTERRUPTED:
        origin = completion.interrupt_origin or InterruptOrigin.EXTERNAL_OR_UNKNOWN
        append_item(
            ctx,
            items,
            event,
            kind="aborted",
            role="assistant",
            phase="native_interrupted",
            text=f"Codex turn interrupted.\nOrigin: {origin.value}",
        )
        return True
    if completion.status is TurnStatus.FAILED:
        append_item(
            ctx,
            items,
            event,
            kind="failed",
            role="assistant",
            phase="native_failed",
            text=completion.error_message or "Codex turn failed without an error message.",
        )
        return True
    return False


def _append_completed_turn(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    payload: SessionPayload,
    turn_id: str,
) -> None:
    if ctx.turn_completion_error:
        append_item(
            ctx,
            items,
            event,
            kind="transport_error",
            role="assistant",
            phase="terminal_reconciliation_failed",
            text=ctx.turn_completion_error,
        )
        return
    if ctx.goal_lookup_error and turn_id not in ctx.goal_updates:
        append_item(
            ctx,
            items,
            event,
            kind="transport_error",
            role="assistant",
            phase="goal_lookup_failed",
            text=ctx.goal_lookup_error,
        )
        return
    text = str(payload.get("last_agent_message") or "").strip()
    if not text:
        append_no_visible_reply_if_needed(ctx, items, event, payload)
        return
    is_final = is_final_goal_turn(ctx, turn_id)
    append_agent_if_new(
        ctx,
        items,
        event,
        text,
        kind="final" if is_final else "commentary",
        phase="final_answer" if is_final else "goal_turn_complete",
    )


def contains_task_complete_event(events: list[SessionEvent]) -> bool:
    for event in events:
        payload = event_payload(event)
        if payload is not None and event.get("type") == "event_msg" and payload.get("type") == "task_complete":
            return True
    return False


def terminal_turn_ids(events: list[SessionEvent]) -> list[str]:
    turn_ids: list[str] = []
    for event in events:
        payload = event_payload(event)
        if payload is None or event.get("type") != "event_msg":
            continue
        if payload.get("type") not in ABORTED_PAYLOAD_TYPES | {"task_complete"}:
            continue
        turn_id = payload_turn_id(payload)
        if turn_id and turn_id not in turn_ids:
            turn_ids.append(turn_id)
    return turn_ids


def latest_terminal_turn_id(events: list[SessionEvent]) -> str | None:
    latest: str | None = None
    for event in events:
        payload = event_payload(event)
        if payload is None or event.get("type") != "event_msg":
            continue
        if payload.get("type") not in ABORTED_PAYLOAD_TYPES | {"task_complete"}:
            continue
        turn_id = payload_turn_id(payload)
        if turn_id:
            latest = turn_id
    return latest
