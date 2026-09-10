from __future__ import annotations

import os
from pathlib import Path

import codex_desktop_bridge_thread_store as thread_store
import codex_discord_store as discord_store
from codex_thread_models import ThreadInfo
from send_discord_attachment_types import AttachmentCliArgs, AttachmentTargetError

SCRIPT_DIR = Path(__file__).resolve().parent
MIRROR_DB_ENV = "CODEX_DISCORD_MIRROR_DB"
DEFAULT_MIRROR_DB_PATH = SCRIPT_DIR / "discord_mirror.sqlite"


def mirror_db_path() -> Path:
    configured = os.environ.get(MIRROR_DB_ENV)
    return Path(configured).expanduser() if configured else DEFAULT_MIRROR_DB_PATH


def resolve_thread_from_ref(thread_ref: str) -> ThreadInfo:
    try:
        return thread_store.resolve_thread_ref(thread_ref)
    except thread_store.ThreadStoreError as active_error:
        try:
            return thread_store.resolve_archived_thread_ref(thread_ref)
        except thread_store.ThreadStoreError as archived_error:
            raise AttachmentTargetError(
                f"not in active/archived threads: {thread_ref}; "
                + f"active={active_error}; archived={archived_error}"
            ) from archived_error


def resolve_mirrored_channel_id(thread_ref: str) -> str:
    thread = resolve_thread_from_ref(thread_ref)
    row = discord_store.get_mirror_thread_row_by_codex_thread_id(mirror_db_path(), thread.id)
    if row is None:
        raise AttachmentTargetError(
            f"stale mirror mapping: no Discord thread mapping for {thread.id}; "
            + "run !mirror check, then !mirror sync"
        )
    return str(int(row[1]))


def resolve_target_channel_id(args: AttachmentCliArgs) -> tuple[str, bool]:
    if args.channel_id:
        return str(args.channel_id), False
    target_ref = args.thread_ref or args.work_thread
    if not target_ref:
        raise AttachmentTargetError("missing Discord target: use --channel-id, --thread-ref, or --work-thread")
    return resolve_mirrored_channel_id(target_ref), True
