from __future__ import annotations

import codex_discord_session_mirror_activity_items as activity_items
import codex_discord_session_mirror_function_items as function_items
import codex_discord_session_mirror_item_builders as item_builders
import codex_discord_session_mirror_item_events as item_events
from codex_discord_session_mirror_detail import SessionMirrorDetailMode
from codex_discord_session_mirror_item_append import (
    BuildInteractiveNoticeFunc,
    CollectionContext,
    ExtractMessageTextFunc,
    SessionPayload,
    SkipDiscordOriginPromptFunc,
    append_agent_if_new as _append_agent_if_new,
    append_item as _append_item,
    append_user_if_new as _append_user_if_new,
)
from codex_app_server_transport_goal import ThreadGoalStatus, ThreadGoalUpdate
from codex_app_server_transport_turn_outcomes import TurnCompletion, TurnStatus
from codex_discord_session_mirror_item_builders import (
    SessionEvent,
    SessionMirrorItem,
    TextDigestFunc,
)

INTERNAL_RESPONSE_USER_PREFIXES = (
    "# AGENTS.md instructions",
    "<INSTRUCTIONS>",
    "<environment_context",
    "<codex_internal_context",
)


def _collect_response_message(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    payload: SessionPayload,
) -> None:
    text = ctx.extract_message_text(payload)
    if not text:
        return
    role = str(payload.get("role") or "?")
    phase = str(payload.get("phase") or "")
    if role == "assistant" and phase == "commentary":
        _append_agent_if_new(ctx, items, event, text, kind="commentary", phase=phase)
        return
    if role == "assistant" and phase == "final_answer":
        return
    if role == "user" and not text.lstrip().startswith(INTERNAL_RESPONSE_USER_PREFIXES):
        _append_user_if_new(ctx, items, event, text, phase)


def _collect_response_item(
    ctx: CollectionContext,
    items: list[SessionMirrorItem],
    event: SessionEvent,
    payload: SessionPayload,
    detail_mode: SessionMirrorDetailMode,
) -> None:
    payload_type = str(payload.get("type") or "")
    if detail_mode is SessionMirrorDetailMode.ALL:
        for activity_index, activity_item in enumerate(
            activity_items.build_activity_items(payload)
        ):
            _append_item(
                ctx,
                items,
                event,
                kind="commentary",
                role="assistant",
                phase=activity_item.phase,
                text=activity_item.text,
            )
            item = items[-1]
            item["text"] = activity_item.text
            item["digest"] = ctx.make_text_digest(
                item["digest"],
                activity_item.text,
                str(activity_index),
            )
    function_item_sink = function_items.FunctionItemSink(
        ctx=ctx,
        items=items,
        event=event,
    )
    if function_items.collect_function_item(function_item_sink, payload):
        return
    if payload_type == "message":
        _collect_response_message(ctx, items, event, payload)


def collect_session_mirror_items(
    codex_thread_id: str,
    events: list[SessionEvent],
    *,
    seen_agent_messages: dict[str, float],
    seen_user_messages: dict[str, float],
    should_skip_discord_origin_prompt_func: SkipDiscordOriginPromptFunc,
    build_interactive_notice_func: BuildInteractiveNoticeFunc,
    extract_message_text_func: ExtractMessageTextFunc,
    recent_text_ttl_seconds: float,
    goal_status: ThreadGoalStatus | None = None,
    turn_completions: dict[str, TurnCompletion] | None = None,
    turn_completion_error: str = "",
    goal_updates: dict[str, ThreadGoalUpdate] | None = None,
    goal_lookup_error: str = "",
    make_text_digest_func: TextDigestFunc = item_builders.make_text_digest,
    detail_mode: SessionMirrorDetailMode = SessionMirrorDetailMode.SEND,
) -> list[SessionMirrorItem]:
    completion_map = {} if turn_completions is None else turn_completions
    latest_native_completed_turn_id = next(
        (
            turn_id
            for turn_id, completion in reversed(list(completion_map.items()))
            if completion.status is TurnStatus.COMPLETED
        ),
        None,
    )
    ctx = CollectionContext(
        codex_thread_id=codex_thread_id,
        seen_agent_messages=seen_agent_messages,
        seen_user_messages=seen_user_messages,
        should_skip_discord_origin_prompt=should_skip_discord_origin_prompt_func,
        build_interactive_notice=build_interactive_notice_func,
        extract_message_text=extract_message_text_func,
        recent_text_ttl_seconds=recent_text_ttl_seconds,
        make_text_digest=make_text_digest_func,
        goal_status=goal_status,
        latest_terminal_turn_id=latest_native_completed_turn_id
        or item_events.latest_terminal_turn_id(events),
        terminal_turn_ids=set(),
        turn_completions=completion_map,
        turn_completion_error=turn_completion_error,
        goal_updates={} if goal_updates is None else goal_updates,
        goal_lookup_error=goal_lookup_error,
    )
    items: list[SessionMirrorItem] = []
    for event in events:
        payload = item_events.event_payload(event)
        if payload is None:
            continue
        event_type = str(event.get("type") or "")
        if event_type == "event_msg":
            item_events.collect_event_message(ctx, items, event, payload)
        elif event_type == "response_item":
            _collect_response_item(ctx, items, event, payload, detail_mode)
    return items
