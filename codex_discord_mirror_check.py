from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass, replace

import codex_discord_mirror_access as mirror_access
from codex_discord_mirror_rows import MirrorCheckRow
from codex_thread_models import ThreadInfo


@dataclass(frozen=True, slots=True)
class MirrorCheckExpectedThread:
    thread_id: str
    project_key: str
    project_name: str
    title: str


@dataclass(frozen=True, slots=True)
class MirrorCheckWrongProject:
    thread_id: str
    current_project_key: str
    expected_project_key: str
    row: MirrorCheckRow


@dataclass(frozen=True, slots=True)
class MirrorCheckSummary:
    expected_count: int
    mirrored_count: int
    missing: tuple[MirrorCheckExpectedThread, ...]
    stale: tuple[MirrorCheckRow, ...]
    wrong_project: tuple[MirrorCheckWrongProject, ...]


def build_mirror_check_expected_threads(
    threads: Sequence[ThreadInfo],
    *,
    get_project_key_func: Callable[[ThreadInfo], str],
    get_project_name_func: Callable[[ThreadInfo], str],
    get_thread_ui_name_func: Callable[[str, ThreadInfo | None], str],
) -> tuple[MirrorCheckExpectedThread, ...]:
    expected: list[MirrorCheckExpectedThread] = []
    for thread in threads:
        expected.append(
            MirrorCheckExpectedThread(
                thread_id=thread.id,
                project_key=get_project_key_func(thread),
                project_name=get_project_name_func(thread),
                title=get_thread_ui_name_func(thread.id, thread) or thread.title or thread.id[:8],
            )
        )
    return tuple(expected)


def summarize_mirror_check(
    expected_threads: Sequence[MirrorCheckExpectedThread],
    parsed_rows: Sequence[MirrorCheckRow],
) -> MirrorCheckSummary:
    expected_by_id: dict[str, MirrorCheckExpectedThread] = {}
    for expected in expected_threads:
        expected_by_id[expected.thread_id] = expected

    rows = tuple(parsed_rows)
    mirrored_thread_ids = {row.codex_thread_id for row in rows}
    missing = tuple(
        expected_by_id[thread_id] for thread_id in expected_by_id if thread_id not in mirrored_thread_ids
    )
    stale = tuple(
        replace(
            row,
            stale=True,
            reason=(
                row.reason
                if row.stale or row.reason != mirror_access.ACTIVE_MAPPING_REASON
                else mirror_access.not_in_active_or_archived_thread_lists_reason()
            ),
        )
        for row in rows
        if row.codex_thread_id not in expected_by_id
    )
    wrong_project = tuple(
        MirrorCheckWrongProject(
            thread_id=row.codex_thread_id,
            current_project_key=row.project_key,
            expected_project_key=expected_by_id[row.codex_thread_id].project_key,
            row=row,
        )
        for row in rows
        if row.codex_thread_id in expected_by_id
        and row.project_key != expected_by_id[row.codex_thread_id].project_key
    )
    return MirrorCheckSummary(
        expected_count=len(expected_by_id),
        mirrored_count=len(rows),
        missing=missing,
        stale=stale,
        wrong_project=wrong_project,
    )


def format_mirror_check_summary(
    summary: MirrorCheckSummary,
    *,
    archive_recommended_count: int | None = None,
    app_server_unavailable_count: int = 0,
) -> str:
    lines = [
        "Mirror check",
        (
            "This checks Codex-to-mirror DB mappings and app-server thread availability."
            if app_server_unavailable_count
            else "This checks Codex-to-mirror DB mappings only."
        ),
        "`!mirror sync` removes stale/orphan threads only under Codex mirror project channels.",
        "General Discord threads are outside this check.",
        "`rec archive` is only a recommendation; archive first, then sync.",
        f"codex_threads: {summary.expected_count}",
        *(
            [f"app_server_unavailable_threads: {app_server_unavailable_count}"]
            if app_server_unavailable_count
            else []
        ),
        f"mirrored_threads: {summary.mirrored_count}",
        f"missing: {len(summary.missing)}",
        f"stale: {len(summary.stale)}",
        f"wrong_project: {len(summary.wrong_project)}",
    ]
    if archive_recommended_count is not None:
        lines.append(f"archive_recommended: {archive_recommended_count}")
    if summary.missing:
        lines.append("")
        lines.append("Missing:")
        for thread in summary.missing[:10]:
            lines.append(f"- {thread.project_name} / {thread.title} ({thread.thread_id[:8]})")
    if summary.wrong_project:
        lines.append("")
        lines.append("Wrong project:")
        for row in summary.wrong_project[:10]:
            lines.append(
                f"- {row.thread_id[:8]} current={row.current_project_key} expected={row.expected_project_key}"
                + _format_mirror_check_row_visibility(row.row)
            )
    if summary.stale:
        lines.append("")
        lines.append("Stale:")
        for row in summary.stale[:10]:
            lines.append(f"- {row.codex_thread_id[:8]}" + _format_mirror_check_row_visibility(row))
    if summary.missing or summary.wrong_project or summary.stale:
        lines.append("")
        lines.append("Run `!mirror sync` to repair.")
    return "\n".join(lines)


def _format_mirror_check_row_visibility(row: MirrorCheckRow) -> str:
    return (
        f" discord_thread_id={row.discord_thread_id}"
        + f" parent_channel_id={row.parent_channel_id}"
        + f" accessible={row.accessible}"
        + f" archived={row.archived}"
        + f" last_seen={row.last_seen}"
        + f" stale={mirror_access.bool_label(row.stale)}"
        + f" reason={row.reason}"
    )
