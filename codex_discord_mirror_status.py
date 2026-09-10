"""Mirror status/list message builders for the Discord bridge."""

from __future__ import annotations

import sqlite3
from dataclasses import replace
from pathlib import Path
from collections.abc import Mapping
from typing import Callable, Protocol

import codex_discord_mirror_access as mirror_access
from codex_discord_store_connection import connect_store
from codex_discord_mirror_check import (
    MirrorCheckExpectedThread,
    build_mirror_check_expected_threads,
    format_mirror_check_summary,
    summarize_mirror_check,
)
from codex_discord_mirror_rows import (
    MirrorCheckRow,
    MirrorListRow,
    mirror_check_row_from_db,
    mirror_list_row_from_db,
)
from codex_discord_mirror_status_queries import load_mirror_check_rows, load_mirror_list_rows
from codex_thread_models import ThreadContextUsage, ThreadInfo


MirrorAccessStatusMap = Mapping[int, mirror_access.MirrorThreadAccessStatus]


class MirrorListBridge(Protocol):
    choose_thread: Callable[[str, str | None], ThreadInfo]
    get_thread_context_usage: Callable[[ThreadInfo], ThreadContextUsage | None]
    describe_thread_context_usage: Callable[[ThreadContextUsage], str]
    should_recommend_archive: Callable[[ThreadInfo, ThreadContextUsage | None], bool]
    get_thread_collaboration_mode: Callable[[ThreadInfo], str]
    get_thread_service_tier: Callable[[ThreadInfo], str]
    format_thread_model_display: Callable[[ThreadInfo, str, str], str]


class MirrorCheckBridge(Protocol):
    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]: ...

    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str: ...


def _mirror_thread_detail_suffix(codex_thread_id: str, bridge_module: MirrorListBridge) -> str:
    thread = bridge_module.choose_thread(codex_thread_id, None)
    context_usage = bridge_module.get_thread_context_usage(thread)
    context_suffix = ""
    if context_usage is not None:
        status = bridge_module.describe_thread_context_usage(context_usage)
        archive_hint = " archive" if bridge_module.should_recommend_archive(thread, context_usage) else ""
        context_suffix = f" ctx={context_usage.usage_ratio * 100:.1f}%/{status}{archive_hint}"
    model_display = bridge_module.format_thread_model_display(
        thread,
        bridge_module.get_thread_collaboration_mode(thread),
        bridge_module.get_thread_service_tier(thread),
    )
    return f" model {model_display}{context_suffix}"


def _mirror_thread_error_suffix(exc: RuntimeError) -> str:
    message = str(exc).splitlines()[0].strip()
    if not message:
        message = type(exc).__name__
    return f" meta_error={type(exc).__name__}: {message}"


def _apply_mirror_list_row_status(
    row: MirrorListRow,
    access_statuses: MirrorAccessStatusMap | None,
) -> MirrorListRow:
    if access_statuses is None:
        return row
    status = access_statuses.get(row.discord_thread_id)
    if status is None:
        return row
    return replace(
        row,
        accessible=status.accessible,
        archived=status.archived,
        stale=status.stale,
        reason=status.reason,
    )


def _apply_mirror_check_row_status(
    row: MirrorCheckRow,
    access_statuses: MirrorAccessStatusMap | None,
) -> MirrorCheckRow:
    if access_statuses is None:
        return row
    status = access_statuses.get(row.discord_thread_id)
    if status is None:
        return row
    return replace(
        row,
        accessible=status.accessible,
        archived=status.archived,
        stale=status.stale,
        reason=status.reason,
    )


def _format_mirror_list_row(row: MirrorListRow, bridge_module: MirrorListBridge) -> str:
    detail_suffix = ""
    try:
        detail_suffix = _mirror_thread_detail_suffix(row.codex_thread_id, bridge_module)
    except RuntimeError as exc:
        detail_suffix = _mirror_thread_error_suffix(exc)
    return (
        f"- {row.project_name or '-'} / {row.title or row.codex_thread_id[:8]} "
        + f"=> <#{row.discord_thread_id}> ({row.codex_thread_id[:8]})"
        + _format_mirror_list_visibility(row)
        + detail_suffix
    )


def _format_mirror_list_visibility(row: MirrorListRow) -> str:
    return (
        f" discord_thread_id={row.discord_thread_id}"
        + f" parent_channel_id={row.parent_channel_id}"
        + f" accessible={row.accessible}"
        + f" archived={row.archived}"
        + f" last_seen={row.last_seen}"
        + f" stale={mirror_access.bool_label(row.stale)}"
        + f" reason={row.reason}"
    )


def build_mirror_list(
    limit: int,
    *,
    scoped_thread_ids: list[str] | None = None,
    db_path: Path,
    init_mirror_db_func: Callable[[], None],
    bridge_module: MirrorListBridge,
    access_statuses: MirrorAccessStatusMap | None = None,
) -> str:
    init_mirror_db_func()
    with connect_store(db_path) as conn:
        conn.row_factory = sqlite3.Row
        rows = load_mirror_list_rows(conn, limit, scoped_thread_ids)
    parsed_rows = [
        _apply_mirror_list_row_status(mirror_list_row_from_db(row), access_statuses)
        for row in rows
    ]
    if not parsed_rows:
        return "No mirrored threads yet. Run `!mirror sync`."
    lines = ["Mirrored Codex threads"]
    for row in parsed_rows:
        lines.append(_format_mirror_list_row(row, bridge_module))
    return "\n".join(lines)


def _resolve_mirror_check_threads(
    threads: list[ThreadInfo] | None,
    limit: int | None,
    bridge_module: MirrorCheckBridge,
) -> list[ThreadInfo]:
    if threads is not None:
        return threads
    if limit is None:
        raise RuntimeError("Mirror check requires explicit threads or an explicit limit.")  # noqa: GENERIC_ERR_OK
    return bridge_module.load_recent_threads(limit=limit)


def _build_expected_mirror_check_threads(
    threads: list[ThreadInfo],
    *,
    bridge_module: MirrorCheckBridge,
    filter_mirrorable_threads_func: Callable[[list[ThreadInfo]], list[ThreadInfo]],
    get_project_key_func: Callable[[ThreadInfo], str],
    get_project_name_func: Callable[[ThreadInfo], str],
) -> tuple[MirrorCheckExpectedThread, ...]:
    mirrorable_threads = filter_mirrorable_threads_func(threads)
    return build_mirror_check_expected_threads(
        mirrorable_threads,
        get_project_key_func=get_project_key_func,
        get_project_name_func=get_project_name_func,
        get_thread_ui_name_func=bridge_module.get_thread_ui_name,
    )


def build_mirror_check(
    *,
    threads: list[ThreadInfo] | None = None,
    limit: int | None = None,
    db_path: Path,
    init_mirror_db_func: Callable[[], None],
    bridge_module: MirrorCheckBridge,
    filter_mirrorable_threads_func: Callable[[list[ThreadInfo]], list[ThreadInfo]],
    get_project_key_func: Callable[[ThreadInfo], str],
    get_project_name_func: Callable[[ThreadInfo], str],
    archive_recommended_count: int | None = None,
    app_server_unavailable_count: int = 0,
    scoped_project_keys: set[str] | None = None,
    access_statuses: MirrorAccessStatusMap | None = None,
) -> str:
    init_mirror_db_func()
    resolved_threads = _resolve_mirror_check_threads(threads, limit, bridge_module)
    expected = _build_expected_mirror_check_threads(
        resolved_threads,
        bridge_module=bridge_module,
        filter_mirrorable_threads_func=filter_mirrorable_threads_func,
        get_project_key_func=get_project_key_func,
        get_project_name_func=get_project_name_func,
    )

    with connect_store(db_path) as conn:
        conn.row_factory = sqlite3.Row
        rows = load_mirror_check_rows(conn, scoped_project_keys)

    parsed_rows = [
        _apply_mirror_check_row_status(mirror_check_row_from_db(row), access_statuses)
        for row in rows
    ]
    summary = summarize_mirror_check(expected, parsed_rows)
    return format_mirror_check_summary(
        summary,
        archive_recommended_count=archive_recommended_count,
        app_server_unavailable_count=app_server_unavailable_count,
    )


def load_mirror_list_access_targets(
    limit: int,
    *,
    scoped_thread_ids: list[str] | None,
    db_path: Path,
    init_mirror_db_func: Callable[[], None],
) -> list[MirrorListRow]:
    init_mirror_db_func()
    with connect_store(db_path) as conn:
        conn.row_factory = sqlite3.Row
        rows = load_mirror_list_rows(conn, limit, scoped_thread_ids)
    return [mirror_list_row_from_db(row) for row in rows]


def load_mirror_check_access_targets(
    *,
    db_path: Path,
    init_mirror_db_func: Callable[[], None],
    scoped_project_keys: set[str] | None = None,
) -> list[MirrorCheckRow]:
    init_mirror_db_func()
    with connect_store(db_path) as conn:
        conn.row_factory = sqlite3.Row
        rows = load_mirror_check_rows(conn, scoped_project_keys)
    return [mirror_check_row_from_db(row) for row in rows]
