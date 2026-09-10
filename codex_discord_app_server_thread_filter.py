from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

import codex_app_server_transport as app_server_transport
from codex_app_server_transport_replies import CodexAppServerTransportError, JsonObject
from codex_discord_logging import log_line
from codex_discord_text import env_flag
from codex_thread_models import ThreadInfo


class AppServerThreadReader(Protocol):
    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject: ...

    def try_restart_if_quiescent(self) -> bool: ...


@dataclass(frozen=True, slots=True)
class AppServerThreadFilterDeps:
    app_server_transport_enabled: Callable[[], bool]
    get_client: Callable[[], AppServerThreadReader]
    log: Callable[[str], None]


def _is_thread_not_found_error(exc: Exception) -> bool:
    return "Thread not found:" in str(exc)


def _can_read_thread(client: AppServerThreadReader, thread_id: str) -> bool:
    try:
        _ = client.read_thread(thread_id, include_turns=False)
    except CodexAppServerTransportError as exc:
        if _is_thread_not_found_error(exc):
            return False
        raise
    return True


def filter_app_server_available_threads_with_deps(
    threads: list[ThreadInfo],
    *,
    deps: AppServerThreadFilterDeps,
) -> list[ThreadInfo]:
    if not deps.app_server_transport_enabled():
        return threads

    client = deps.get_client()
    refresh_attempted = False
    available: list[ThreadInfo] = []
    for thread in threads:
        if _can_read_thread(client, thread.id):
            available.append(thread)
            continue
        if not refresh_attempted:
            refresh_attempted = True
            deps.log(f"mirror_sync_app_server_refresh_start target={thread.id}")
            if not client.try_restart_if_quiescent():
                deps.log(f"mirror_sync_app_server_refresh_deferred target={thread.id}")
                continue
            if _can_read_thread(client, thread.id):
                deps.log(f"mirror_sync_app_server_refresh_recovered target={thread.id}")
                available.append(thread)
                continue
        deps.log(f"mirror_sync_app_server_thread_unavailable target={thread.id} error=thread_not_found")
    return available


def filter_app_server_available_threads(threads: list[ThreadInfo]) -> list[ThreadInfo]:
    return filter_app_server_available_threads_with_deps(
        threads,
        deps=AppServerThreadFilterDeps(
            app_server_transport_enabled=lambda: env_flag("CODEX_DISCORD_APP_SERVER_TRANSPORT", True),
            get_client=lambda: app_server_transport.DEFAULT_CLIENT,
            log=log_line,
        ),
    )
