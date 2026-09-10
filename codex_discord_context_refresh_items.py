from __future__ import annotations

from collections.abc import Callable
from typing import Protocol

from codex_discord_context_refresh_format import (
    format_context_refresh_item as format_context_refresh_item,
    truncate_context_refresh_text as truncate_context_refresh_text,
)
from codex_session_events import JsonEvent, JsonValue

BuildInteractiveNoticeFunc = Callable[[dict[str, JsonValue]], str]
ExtractMessageTextFunc = Callable[[dict[str, JsonValue]], str]


class MakeTextDigestFunc(Protocol):
    def __call__(self, *parts: str) -> str: ...


class MakeSessionMirrorItemFunc(Protocol):
    def __call__(
        self,
        codex_thread_id: str,
        event: JsonEvent,
        *,
        kind: str,
        role: str,
        phase: str,
        text: str,
    ) -> dict[str, str]: ...


def extract_context_refresh_item(
    codex_thread_id: str,
    event: JsonEvent,
    *,
    make_session_mirror_item_func: MakeSessionMirrorItemFunc,
    build_interactive_notice_func: BuildInteractiveNoticeFunc,
    extract_message_text_func: ExtractMessageTextFunc,
) -> dict[str, str] | None:
    payload = event.get("payload") or {}
    if not isinstance(payload, dict):
        return None

    event_type = str(event.get("type") or "")
    payload_type = str(payload.get("type") or "")
    if event_type == "event_msg":
        if payload_type == "user_message":
            text = str(payload.get("message") or "").strip()
            if not text:
                return None
            return make_session_mirror_item_func(
                codex_thread_id,
                event,
                kind="user",
                role="user",
                phase="input",
                text=text,
            )
        if payload_type == "agent_message":
            text = str(payload.get("message") or "").strip()
            if not text:
                return None
            phase = str(payload.get("phase") or "commentary")
            return make_session_mirror_item_func(
                codex_thread_id,
                event,
                kind="final" if phase == "final_answer" else "commentary",
                role="assistant",
                phase=phase,
                text=text,
            )
        return None

    if event_type != "response_item":
        return None
    if payload_type == "function_call":
        notice = build_interactive_notice_func(payload)
        if not notice:
            return None
        return make_session_mirror_item_func(
            codex_thread_id,
            event,
            kind="interactive",
            role="assistant",
            phase="interactive",
            text=notice,
        )
    if payload_type != "message":
        return None

    text = extract_message_text_func(payload)
    if not text:
        return None
    role = str(payload.get("role") or "?")
    phase = str(payload.get("phase") or ("input" if role == "user" else ""))
    kind = "user" if role == "user" else "final" if phase == "final_answer" else "commentary"
    return make_session_mirror_item_func(
        codex_thread_id,
        event,
        kind=kind,
        role=role,
        phase=phase,
        text=text,
    )


def collect_context_refresh_items(
    codex_thread_id: str,
    events: list[JsonEvent],
    *,
    make_session_mirror_item_func: MakeSessionMirrorItemFunc,
    build_interactive_notice_func: BuildInteractiveNoticeFunc,
    extract_message_text_func: ExtractMessageTextFunc,
    make_text_digest_func: MakeTextDigestFunc,
) -> list[dict[str, str]]:
    items: list[dict[str, str]] = []
    seen: set[str] = set()
    for event in events:
        item = extract_context_refresh_item(
            codex_thread_id,
            event,
            make_session_mirror_item_func=make_session_mirror_item_func,
            build_interactive_notice_func=build_interactive_notice_func,
            extract_message_text_func=extract_message_text_func,
        )
        if item is None:
            continue
        text = str(item.get("text") or "").strip()
        if not text:
            continue
        digest = make_text_digest_func(
            "context-refresh",
            item.get("kind") or "",
            item.get("role") or "",
            item.get("phase") or "",
            text,
        )
        if digest in seen:
            continue
        seen.add(digest)
        items.append(item)
    return items
