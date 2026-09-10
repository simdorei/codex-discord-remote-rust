from __future__ import annotations

import json
import mimetypes
from pathlib import Path

from aiohttp import ClientSession, FormData

from send_discord_attachment_types import (
    AttachmentCliArgs,
    DiscordChannelAccessError,
    DiscordMessageResponse,
)


def get_message_content(args: AttachmentCliArgs) -> str:
    if args.content_file:
        return Path(args.content_file).read_text(encoding="utf-8").strip()
    return str(args.content or "").strip()


def build_attachment_form(args: AttachmentCliArgs, files: list[Path]) -> FormData:
    form = FormData()
    payload = {
        "content": get_message_content(args),
        "attachments": [{"id": index, "filename": path.name} for index, path in enumerate(files)],
    }
    form.add_field(
        "payload_json",
        json.dumps(payload, ensure_ascii=False),
        content_type="application/json; charset=utf-8",
    )
    for index, path in enumerate(files):
        form.add_field(
            f"files[{index}]",
            path.read_bytes(),
            filename=path.name,
            content_type=mimetypes.guess_type(path.name)[0] or "application/octet-stream",
        )
    return form


def access_error_message(*, channel_id: str, status: int, response_text: str, mirror_target: bool) -> str:
    label = f"HTTP {status}"
    if status == 403:
        label = "Forbidden"
    elif status == 404 and "Unknown Channel" in response_text:
        label = "Unknown Channel"
    elif status == 404:
        label = "NotFound"

    if mirror_target:
        return (
            f"{label}: stale mirror mapping for Discord channel {channel_id}; "
            + "mirror mapping stale: run !mirror check, then !mirror sync"
        )
    return f"{label}: Discord channel {channel_id} is not accessible"


async def validate_channel_access(
    session: ClientSession,
    *,
    channel_id: str,
    headers: dict[str, str],
    mirror_target: bool,
) -> None:
    url = f"https://discord.com/api/v10/channels/{channel_id}"
    async with session.get(url, headers=headers) as response:
        text = await response.text()
        if 200 <= response.status < 300:
            return
        raise DiscordChannelAccessError(
            status=response.status,
            response_text=text[:1000],
            message=access_error_message(
                channel_id=channel_id,
                status=response.status,
                response_text=text,
                mirror_target=mirror_target,
            ),
        )


def attachment_filenames(result: DiscordMessageResponse) -> list[str]:
    attachments = result.get("attachments", [])
    if not isinstance(attachments, list):
        return []
    filenames: list[str] = []
    for attachment in attachments:
        if isinstance(attachment, dict):
            filenames.append(str(attachment.get("filename") or ""))
    return filenames
