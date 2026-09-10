from __future__ import annotations

from pathlib import Path

import codex_desktop_bridge_session_files as session_files
from codex_app_server_transport_goal import GoalTransportError, ThreadGoalLookup, ThreadGoalStatus, ThreadGoalUpdate
from codex_app_server_transport_turn_outcomes import TurnCompletionObservation, TurnCompletionTransportError


def get_thread_goal_status(session_path: Path) -> ThreadGoalStatus | None:
    import codex_app_server_transport as app_server_transport

    thread_id = _session_thread_id(session_path)
    if thread_id is None:
        return None
    return app_server_transport.DEFAULT_CLIENT.get_thread_goal_status(thread_id)


def observe_turn_completion(session_path: Path, turn_id: str) -> TurnCompletionObservation:
    import codex_app_server_transport as app_server_transport

    thread_id = _session_thread_id(session_path)
    if thread_id is None:
        return TurnCompletionTransportError("Session path has no Codex thread id.")
    return app_server_transport.DEFAULT_CLIENT.observe_turn_completion(thread_id, turn_id)


def get_thread_goal_lookup(session_path: Path) -> ThreadGoalLookup:
    import codex_app_server_transport as app_server_transport

    thread_id = _session_thread_id(session_path)
    if thread_id is None:
        return GoalTransportError("Session path has no Codex thread id.")
    return app_server_transport.DEFAULT_CLIENT.get_thread_goal_lookup(thread_id)


def get_thread_goal_update(session_path: Path, turn_id: str) -> ThreadGoalUpdate | None:
    import codex_app_server_transport as app_server_transport

    thread_id = _session_thread_id(session_path)
    if thread_id is None:
        return None
    return app_server_transport.DEFAULT_CLIENT.get_cached_goal_update(thread_id, turn_id)


def _session_thread_id(session_path: Path) -> str | None:
    match = session_files.SESSION_ID_RE.search(session_path.name)
    return None if match is None else match.group(1)
