from __future__ import annotations

from collections.abc import Callable
import hashlib
import time
from typing import Protocol, TypeAlias, override

from codex_session_events import JsonEvent, JsonValue

TextDigestFunc: TypeAlias = Callable[..., str]
SessionEvent: TypeAlias = JsonEvent
SessionPayload: TypeAlias = dict[str, JsonValue]
SessionMirrorItem: TypeAlias = dict[str, str]


class TextDigestPart(Protocol):
    @override
    def __str__(self) -> str: ...


def make_text_digest(*parts: TextDigestPart | None) -> str:
    digest = hashlib.sha256()
    for part in parts:
        digest.update(str(part or "").encode("utf-8", errors="replace"))
        digest.update(b"\0")
    return digest.hexdigest()


def remember_recent_session_text(
    seen: dict[str, float],
    text: str,
    *,
    ttl_seconds: float,
    now_func: Callable[[], float] = time.monotonic,
    make_text_digest_func: TextDigestFunc = make_text_digest,
) -> None:
    current = now_func()
    expired = [
        digest
        for digest, seen_at in seen.items()
        if current - seen_at > ttl_seconds
    ]
    for digest in expired:
        _ = seen.pop(digest, None)
    seen[make_text_digest_func(text.strip())] = current


def has_recent_session_text(
    seen: dict[str, float],
    text: str,
    *,
    ttl_seconds: float,
    now_func: Callable[[], float] = time.monotonic,
    make_text_digest_func: TextDigestFunc = make_text_digest,
) -> bool:
    current = now_func()
    digest = make_text_digest_func(text.strip())
    seen_at = seen.get(digest)
    if seen_at is None:
        return False
    if current - seen_at > ttl_seconds:
        _ = seen.pop(digest, None)
        return False
    return True


def make_session_mirror_event_digest(
    codex_thread_id: str,
    event: SessionEvent,
    kind: str,
    role: str,
    phase: str,
    text: str,
    *,
    make_text_digest_func: TextDigestFunc = make_text_digest,
) -> str:
    payload_value = event.get("payload")
    payload = payload_value if isinstance(payload_value, dict) else {}
    payload_type = payload.get("type") or ""
    return make_text_digest_func(
        "session-mirror",
        codex_thread_id,
        event.get("timestamp") or "",
        event.get("type") or "",
        payload_type or "",
        kind,
        role,
        phase,
        text,
    )


def make_session_mirror_item(
    codex_thread_id: str,
    event: SessionEvent,
    *,
    kind: str,
    role: str,
    phase: str,
    text: str,
    make_text_digest_func: TextDigestFunc = make_text_digest,
) -> SessionMirrorItem:
    clean_text = str(text or "").strip()
    return {
        "digest": make_session_mirror_event_digest(
            codex_thread_id,
            event,
            kind,
            role,
            phase,
            clean_text,
            make_text_digest_func=make_text_digest_func,
        ),
        "kind": kind,
        "role": role,
        "phase": phase,
        "text": clean_text,
    }


def format_aborted_event_text(payload: SessionPayload) -> str:
    payload_type = str(payload.get("type") or "aborted").strip()
    if payload_type == "task_cancelled":
        headline = "Codex task cancelled."
    elif payload_type == "task_aborted":
        headline = "Codex task aborted."
    else:
        headline = "Codex turn aborted."

    details: list[str] = []
    reason = str(payload.get("reason") or "").strip()
    if reason:
        details.append(f"reason={reason}")
    turn_id = str(payload.get("turn_id") or "").strip()
    if turn_id:
        details.append(f"turn_id={turn_id}")
    task_id = str(payload.get("task_id") or "").strip()
    if task_id:
        details.append(f"task_id={task_id}")
    duration_ms = payload.get("duration_ms")
    if duration_ms is not None:
        details.append(f"duration_ms={duration_ms}")

    if not details:
        return headline
    return f"{headline}\nDetails: {', '.join(details)}"


def format_session_mirror_text(item: SessionMirrorItem) -> str:
    kind = item.get("kind") or ""
    text = item.get("text") or ""
    if kind == "commentary":
        return f"In progress\n\n{text}"
    if kind == "user":
        return f"Codex app user\n\n{text}"
    if kind == "final":
        return f"Final\n\n{text}"
    if kind == "aborted":
        return text or "Codex turn aborted."
    if kind == "failed":
        return f"Failed\n\n{text}"
    if kind == "transport_error":
        return f"Transport error\n\n{text}"
    return text
