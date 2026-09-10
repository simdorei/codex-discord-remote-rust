from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class AttachmentTextPreview:
    filename: str
    preview: str


def render_saved_attachment_detail(
    *,
    index: int,
    filename: str,
    destination: Path,
    content_type: str,
    size_bytes: int,
) -> str:
    return "\n".join(
        [
            f"{index}. {filename}",
            f"   path: {destination}",
            f"   content_type: {content_type or '-'}",
            f"   size_bytes: {size_bytes}",
        ]
    )


def render_attachment_prompt(
    base_prompt: str,
    details: Sequence[str],
    previews: Sequence[AttachmentTextPreview],
) -> str:
    if not details:
        return base_prompt

    lines = [
        base_prompt,
        "",
        "Discord attachments saved locally:",
        *details,
    ]
    if previews:
        lines.extend(["", "Attachment text previews:"])
        for preview in previews:
            lines.extend(
                [
                    f"--- {preview.filename} ---",
                    "```text",
                    preview.preview,
                    "```",
                ]
            )
    return "\n".join(lines).strip()
