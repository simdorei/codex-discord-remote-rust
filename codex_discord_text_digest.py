from __future__ import annotations

import hashlib


type TextDigestPart = str | int | float | bool | bytes | bytearray | None


def make_text_digest(*parts: TextDigestPart) -> str:
    digest = hashlib.sha256()
    for part in parts:
        digest.update(str(part or "").encode("utf-8", errors="replace"))
        digest.update(b"\0")
    return digest.hexdigest()
