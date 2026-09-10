from __future__ import annotations

import re
from typing import Final

CODEX_THREAD_ID_PATTERN: Final = (
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
)
EXPLICIT_CODEX_THREAD_RE: Final = re.compile(
    r"\b(?:Codex\s+thread(?:\s+id)?|codex_thread_id|codex\s*/\s*session|target_thread_id|Work\s+thread)\b"
    + rf"\s*(?::|=)?\s*`?(?P<thread_id>{CODEX_THREAD_ID_PATTERN})`?",
    re.IGNORECASE,
)


def extract_explicit_codex_thread_id(content: str) -> str | None:
    match = EXPLICIT_CODEX_THREAD_RE.search(content)
    if match is None:
        return None
    return match.group("thread_id").lower()
