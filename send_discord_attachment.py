from __future__ import annotations

import argparse
import asyncio  # noqa: ANYIO_OK
import json
import os
from collections.abc import Callable
from pathlib import Path

from aiohttp import ClientSession

import codex_desktop_bridge_thread_store as thread_store
import codex_discord_store as discord_store
from codex_thread_models import ThreadInfo
from send_discord_attachment_http import (
    attachment_filenames,
    build_attachment_form,
    get_message_content,
    validate_channel_access,
)
from send_discord_attachment_target import (
    DEFAULT_MIRROR_DB_PATH,
    MIRROR_DB_ENV,
    mirror_db_path,
    resolve_mirrored_channel_id,
    resolve_target_channel_id,
    resolve_thread_from_ref,
)
from send_discord_attachment_types import (
    AttachmentArgNamespace,
    AttachmentCliArgs,
    AttachmentTargetError,
    DiscordChannelAccessError,
    DiscordMessageResponse,
    DiscordSendFailedError,
    MissingDiscordBotTokenError,
    JsonValue,
)

SCRIPT_DIR = Path(__file__).resolve().parent
ENV_PATH = SCRIPT_DIR / ".env"
_decode_json_value: Callable[[str], JsonValue] = json.loads

_mirror_db_path = mirror_db_path
_resolve_thread_from_ref = resolve_thread_from_ref
_resolve_mirrored_channel_id = resolve_mirrored_channel_id


def load_env(path: Path = ENV_PATH) -> dict[str, str]:
    values: dict[str, str] = {}
    if not path.exists():
        return values
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        values[key.strip()] = value.strip().strip('"')
    return values


async def send_discord_attachment(args: AttachmentCliArgs) -> DiscordMessageResponse:
    env = load_env()
    token = os.environ.get("DISCORD_BOT_TOKEN") or env.get("DISCORD_BOT_TOKEN")
    if not token:
        raise MissingDiscordBotTokenError()

    files = [Path(path).resolve() for path in args.files]
    missing = [str(path) for path in files if not path.exists()]
    if missing:
        raise FileNotFoundError(", ".join(missing))

    channel_id, mirror_target = resolve_target_channel_id(args)
    headers = {"Authorization": f"Bot {token}"}
    url = f"https://discord.com/api/v10/channels/{channel_id}/messages"
    async with ClientSession() as session:
        await validate_channel_access(
            session,
            channel_id=channel_id,
            headers=headers,
            mirror_target=mirror_target,
        )
        async with session.post(url, headers=headers, data=build_attachment_form(args, files)) as response:
            text = await response.text()
            if response.status < 200 or response.status >= 300:
                raise DiscordSendFailedError(status=response.status, response_text=text[:1000])
            decoded = _decode_json_value(text)
            return decoded if isinstance(decoded, dict) else {}


def parse_args() -> AttachmentCliArgs:
    parser = argparse.ArgumentParser(description="Send UTF-8 Discord message content with file attachments.")
    target = parser.add_mutually_exclusive_group(required=True)
    _ = target.add_argument("--channel-id", help="Discord channel or thread ID to send into.")
    _ = target.add_argument("--thread-ref", help="Active or archived Codex thread ref whose mirror thread receives files.")
    _ = target.add_argument("--work-thread", help="Codex work thread ID or ref whose mirror thread receives files.")
    _ = parser.add_argument("--content", default="", help="UTF-8 message content.")
    _ = parser.add_argument("--content-file", help="UTF-8 text file to use as message content.")
    _ = parser.add_argument("files", nargs="+", help="One or more files to attach.")
    namespace = AttachmentArgNamespace()
    _ = parser.parse_args(namespace=namespace)
    return AttachmentCliArgs(
        channel_id=namespace.channel_id,
        thread_ref=namespace.thread_ref,
        work_thread=namespace.work_thread,
        content=namespace.content,
        content_file=namespace.content_file,
        files=namespace.files,
    )


def main() -> None:
    result = asyncio.run(send_discord_attachment(parse_args()))
    print("DISCORD_ATTACHMENT_SENT")
    print("message_id=" + str(result.get("id") or ""))
    print("channel_id=" + str(result.get("channel_id") or ""))
    print("attachments=" + ",".join(attachment_filenames(result)))


if __name__ == "__main__":
    main()
