from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Callable
from pathlib import Path
from typing import Protocol, TypeVar

import codex_pro_session_mirror_gate as pro_session_mirror_gate


ThreadT = TypeVar("ThreadT")


class ProSessionMirrorOutputDeps(Protocol[ThreadT]):
    choose_thread: Callable[[str | None, str | None], ThreadT]
    get_thread_rollout_path: Callable[[ThreadT], str]
    update_session_mirror_cursor: Callable[[str, str, int], None]
    log: Callable[[str], None]


async def gate_session_mirror_output(
    deps: ProSessionMirrorOutputDeps[ThreadT],
    codex_thread_id: str,
) -> bool:
    mode = pro_session_mirror_gate.mode(codex_thread_id)
    if mode is pro_session_mirror_gate.GateMode.OPEN:
        return False
    if mode is pro_session_mirror_gate.GateMode.HOLD:
        return True
    try:
        codex_thread = await asyncio.to_thread(
            deps.choose_thread,
            codex_thread_id,
            None,
        )
    except Exception:  # noqa: BROAD_EXCEPT_OK - rejected Pro output must remain closed until readable.
        return True
    session_path = Path(deps.get_thread_rollout_path(codex_thread))
    if not session_path.exists():
        return True
    rollout_size = session_path.stat().st_size
    if not pro_session_mirror_gate.discard_size_is_stable(
        codex_thread_id,
        rollout_size,
    ):
        return True
    await asyncio.to_thread(
        deps.update_session_mirror_cursor,
        codex_thread_id,
        str(session_path),
        rollout_size,
    )
    if session_path.stat().st_size != rollout_size:
        return True
    pro_session_mirror_gate.finish_discard(codex_thread_id)
    deps.log(
        f"pro_session_mirror_output_discarded target={codex_thread_id} cursor={rollout_size}"
    )
    return True
