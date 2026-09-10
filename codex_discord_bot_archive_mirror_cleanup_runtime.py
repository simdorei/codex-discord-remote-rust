from __future__ import annotations

import sqlite3
import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path

import codex_discord_bridge_process as bridge_process
import codex_discord_runtime as discord_runtime
import codex_discord_session_mirror_archive as discord_session_mirror_archive
import codex_discord_store as discord_store


def _parse_bridge_output_value(output: str, key: str) -> str:
    return bridge_process.parse_bridge_output_value(output, key) or ""


@dataclass(frozen=True, slots=True)
class BotArchiveMirrorCleanupRuntimeDeps:
    get_mirror_db_path: Callable[[], Path]
    get_session_mirror_state: Callable[
        [], discord_session_mirror_archive.SessionMirrorStateLike
    ]
    release_session_mirror_output_target: Callable[[str | None], Awaitable[bool]]
    format_log_argv: Callable[[list[str]], str]
    log: discord_session_mirror_archive.LogFunc


@dataclass(frozen=True, slots=True)
class BotArchiveMirrorCleanupRuntime:
    deps: BotArchiveMirrorCleanupRuntimeDeps

    async def cleanup_archived_session_mirror_state(
        self,
        owner: discord_session_mirror_archive.ArchiveMirrorCleanupOwner | None,
        codex_thread_id: str,
    ) -> discord_session_mirror_archive.ArchivedSessionMirrorCleanupCounts:
        return await discord_session_mirror_archive.cleanup_archived_session_mirror_state(
            owner,
            codex_thread_id,
            deps=self.archive_mirror_cleanup_deps(),
        )

    def archive_mirror_cleanup_deps(
        self,
    ) -> discord_session_mirror_archive.ArchiveMirrorCleanupDeps:
        return discord_session_mirror_archive.ArchiveMirrorCleanupDeps(
            delete_archived_mirror_state=lambda codex_thread_id: (
                discord_store.delete_archived_mirror_state(
                    self.deps.get_mirror_db_path(),
                    codex_thread_id,
                )
            ),
            get_session_mirror_state=self.deps.get_session_mirror_state,
            normalize_runner_key=discord_runtime.normalize_runner_key,
            release_session_mirror_output_target=(
                self.deps.release_session_mirror_output_target
            ),
            parse_bridge_output_value=_parse_bridge_output_value,
            format_log_argv=self.deps.format_log_argv,
            exception_types=(
                AttributeError,
                TypeError,
                OSError,
                RuntimeError,
                sqlite3.Error,
            ),
            format_exception=traceback.format_exc,
            log=self.deps.log,
        )

    async def cleanup_archive_mirror_after_bridge_command(
        self,
        owner: discord_session_mirror_archive.ArchiveMirrorCleanupOwner | None,
        argv: list[str],
        exit_code: int,
        output: str,
    ) -> str | None:
        return await discord_session_mirror_archive.cleanup_archive_mirror_after_bridge_command(
            owner,
            argv,
            exit_code,
            output,
            deps=self.archive_mirror_cleanup_deps(),
        )
