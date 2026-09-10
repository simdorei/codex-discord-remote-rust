# pyright: reportAny=false
from __future__ import annotations

import argparse
from pathlib import Path

from codex_desktop_bridge_command_ask_delivery import deliver_prompt
from codex_desktop_bridge_command_ask_types import CommandAskDeps
from codex_desktop_bridge_command_ask_wait import start_background_watch, wait_for_final_answer


class CommandAskError(RuntimeError):
    pass


def run_command_ask(args: argparse.Namespace, *, deps: CommandAskDeps) -> int:
    thread = deps.choose_thread(args.thread_id, args.cwd)
    session_path = Path(thread.rollout_path)
    if not session_path.exists():
        raise CommandAskError(f"Session file not found: {session_path}")

    prompt = args.prompt
    print(f"target_thread: {thread.id}")
    print(f"title: {deps.format_title_preview(thread.title)}")
    print(f"ui_name: {deps.get_thread_ui_name(thread.id, thread) or '-'}")
    print(f"cwd: {thread.cwd}")
    print("")

    if args.dry_run:
        print("[dry_run]")
        print(prompt)
        return 0

    busy_state = deps.get_thread_busy_state(thread, allow_resume=True)
    use_sidecar = bool(getattr(args, "sidecar", False))
    if busy_state != "idle" and not args.force_while_busy and (use_sidecar or not args.ipc):
        raise RuntimeError(deps.describe_thread_busy_state(busy_state))

    start_offset = session_path.stat().st_size
    recent_offsets = deps.snapshot_recent_session_offsets(limit=10, include_threads=[thread])
    sidecar_client = None
    try:
        sidecar_client = deliver_prompt(
            args,
            deps,
            thread,
            prompt,
            recent_offsets,
            use_sidecar=use_sidecar,
        )

        if args.background:
            return start_background_watch(args, deps, thread, start_offset)

        if not args.wait:
            return 0

        return wait_for_final_answer(args, deps, session_path, start_offset)
    finally:
        if sidecar_client is not None:
            sidecar_client.close()
