from __future__ import annotations

from dataclasses import dataclass
from typing import Final

from codex_discord_attachment_metadata import sanitize_attachment_filename
from codex_discord_delivery_runtime import is_attachment_source_allowed
from codex_discord_session_mirror_item_append import (
    CollectionContext,
    SessionPayload,
    append_item,
)
from codex_discord_session_mirror_item_builders import (
    SessionEvent,
    SessionMirrorItem,
)
from codex_session_events import JsonValue

CODEX_IMAGE_OUTPUT_TEXT: Final = "Codex image output"
CODEX_IMAGE_OUTPUT_FILENAME: Final = "codex-image-output.png"
CODEX_FILE_OUTPUT_TEXT: Final = "Codex file output"
CODEX_FILE_OUTPUT_FILENAME: Final = "codex-file-output.bin"
CODEX_FILE_OUTPUT_PART_TYPES: Final = frozenset({"file", "input_file", "output_file"})
CODEX_FILE_DATA_FIELDS: Final = ("file_data", "data_url", "file_url", "url")
CODEX_FILE_PATH_FIELDS: Final = ("file_path", "path")
CODEX_FILE_NAME_FIELDS: Final = ("filename", "download_name", "name")


@dataclass(frozen=True, slots=True)
class FunctionItemSink:
    ctx: CollectionContext
    items: list[SessionMirrorItem]
    event: SessionEvent


def collect_function_item(
    sink: FunctionItemSink,
    payload: SessionPayload,
) -> bool:
    payload_type = str(payload.get("type") or "")
    if payload_type == "function_call":
        notice = sink.ctx.build_interactive_notice(payload)
        if notice:
            append_item(
                sink.ctx,
                sink.items,
                sink.event,
                kind="interactive",
                role="assistant",
                phase="interactive",
                text=notice,
            )
        return True
    if payload_type in {"function_call_output", "custom_tool_call_output"}:
        _collect_output_attachments(sink, payload.get("output"))
        output_text = str(payload.get("output") or "").strip()
        if output_text and "rejected by user" in output_text.lower():
            append_item(
                sink.ctx,
                sink.items,
                sink.event,
                kind="commentary",
                role="assistant",
                phase="approval_rejected",
                text="[approval_rejected]\nCommand approval was rejected by user.",
            )
        return True
    return False


def _collect_output_attachments(
    sink: FunctionItemSink,
    output: JsonValue,
) -> None:
    if not isinstance(output, list):
        return
    for part_index, part in enumerate(output):
        if not isinstance(part, dict):
            continue
        if part.get("type") == "input_image":
            _append_image_item(sink, part, part_index)
            continue
        part_type = str(part.get("type") or "")
        if part_type not in CODEX_FILE_OUTPUT_PART_TYPES:
            continue
        attachment_source = _file_attachment_source(part)
        if attachment_source:
            _append_file_item(
                sink,
                attachment_source,
                _first_string_field(part, CODEX_FILE_NAME_FIELDS),
            )


def _append_image_item(
    sink: FunctionItemSink,
    payload: dict[str, JsonValue],
    image_index: int,
) -> None:
    image_url = payload.get("image_url")
    if not isinstance(image_url, str) or not image_url.startswith("data:image/"):
        return
    append_item(
        sink.ctx,
        sink.items,
        sink.event,
        kind="image",
        role="assistant",
        phase="tool_image",
        text=CODEX_IMAGE_OUTPUT_TEXT,
    )
    item = sink.items[-1]
    item["digest"] = sink.ctx.make_text_digest(item["digest"], str(image_index))
    item["attachment_url"] = image_url
    item["attachment_filename"] = CODEX_IMAGE_OUTPUT_FILENAME


def _append_file_item(
    sink: FunctionItemSink,
    attachment_source: str,
    filename: str,
) -> None:
    source_filename = filename
    if not source_filename and not attachment_source.startswith("data:"):
        source_filename = attachment_source
    safe_filename = sanitize_attachment_filename(
        source_filename or CODEX_FILE_OUTPUT_FILENAME, 1
    )
    append_item(
        sink.ctx,
        sink.items,
        sink.event,
        kind="file",
        role="assistant",
        phase="tool_file",
        text=f"{CODEX_FILE_OUTPUT_TEXT}: {safe_filename}",
    )
    item = sink.items[-1]
    item["attachment_url"] = attachment_source
    item["attachment_filename"] = safe_filename


def _file_attachment_source(payload: dict[str, JsonValue]) -> str:
    attachment_source = _first_string_field(payload, CODEX_FILE_DATA_FIELDS)
    if attachment_source and is_attachment_source_allowed(attachment_source):
        return attachment_source
    if attachment_source:
        return ""
    attachment_source = _first_string_field(payload, CODEX_FILE_PATH_FIELDS)
    if attachment_source and is_attachment_source_allowed(attachment_source):
        return attachment_source
    return ""


def _first_string_field(
    payload: dict[str, JsonValue],
    field_names: tuple[str, ...],
) -> str:
    for field_name in field_names:
        value = payload.get(field_name)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return ""
