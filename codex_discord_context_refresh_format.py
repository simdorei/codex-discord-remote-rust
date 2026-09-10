from __future__ import annotations


def truncate_context_refresh_text(text: str, *, max_chars: int) -> str:
    clean_text = str(text or "").strip()
    max_chars = max(100, int(max_chars))
    if len(clean_text) <= max_chars:
        return clean_text
    marker = "\n[truncated]"
    return clean_text[: max_chars - len(marker)].rstrip() + marker


def format_context_refresh_item(item: dict[str, str], *, max_chars: int) -> str:
    kind = item.get("kind") or ""
    role = item.get("role") or "?"
    phase = item.get("phase") or ""
    if kind == "user":
        label = "user"
    elif kind == "final":
        label = "assistant final"
    elif kind == "interactive":
        label = "assistant interactive"
    elif role == "assistant":
        label = "assistant commentary"
    else:
        label = " ".join(part for part in [role, phase] if part).strip() or "message"
    return f"[{label}]\n{truncate_context_refresh_text(item.get('text') or '', max_chars=max_chars)}"
