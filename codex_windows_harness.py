"""Windows-local frontend harness for Codex app/web bridge decisions.

This module keeps local Codex app facts out of Discord UI code.
It is intentionally small: Discord owns routing and buttons, while this
harness reports structured runtime facts. Ask preflight resolves only the
target thread; idle/busy is not a delivery policy anymore. Codex CLI probing
is diagnostic only; Discord remains the frontend surface.
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, is_dataclass
import json
import shutil
import subprocess
import time
from typing import cast

import codex_desktop_bridge as bridge
from codex_windows_harness_types import (
    AskPreflight,
    DesktopDiscoveryBridge,
    DictExportable,
    HarnessArgs,
    HarnessRuntime,
    HarnessThread,
    JsonObject,
    PrintableJsonData,
    TargetThreadBridge,
    ThreadLabelBridge,
    ThreadRefBridge,
    ThreadSnapshotBridge,
)
from codex_thread_models import ThreadInfo


HARNESS_VERSION = "2026.06.05-1"


def thread_ref(thread: ThreadInfo, *, bridge_module: ThreadRefBridge = bridge) -> str:
    try:
        return bridge_module.get_thread_workspace_ref(thread)
    except Exception:  # noqa: BROAD_EXCEPT_OK
        return thread.id[:8]


def thread_label(thread: ThreadInfo, *, bridge_module: ThreadLabelBridge = bridge) -> str:
    try:
        return bridge_module.get_thread_label(thread)
    except Exception:  # noqa: BROAD_EXCEPT_OK
        return thread.id[:8]


def thread_snapshot(
    thread: ThreadInfo,
    *,
    state: str,
    bridge_module: ThreadSnapshotBridge = bridge,
) -> HarnessThread:
    return HarnessThread(
        id=thread.id,
        ref=thread_label(thread, bridge_module=bridge_module),
        title=thread.title,
        cwd=thread.cwd,
        state=state,
        updated_at=int(thread.updated_at),
    )


def choose_target_thread(
    target_thread_id: str | None,
    *,
    bridge_module: TargetThreadBridge = bridge,
) -> tuple[ThreadInfo | None, str]:
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
    except Exception:  # noqa: BROAD_EXCEPT_OK
        if target_thread_id:
            return None, target_thread_id[:8]
        return None, ""
    return thread, thread_ref(thread, bridge_module=bridge_module)


def preflight_ask(
    target_thread_id: str | None,
    *,
    bridge_module: TargetThreadBridge = bridge,
    now: float | None = None,
) -> AskPreflight:
    checked_at = time.time() if now is None else now
    target_thread, target_ref = choose_target_thread(target_thread_id, bridge_module=bridge_module)
    resolved_target_id = target_thread.id if target_thread is not None else target_thread_id
    target_state = "not_checked"
    events: list[JsonObject] = [
        {
            "type": "preflight_checked",
            "at": checked_at,
            "target_thread_id": resolved_target_id,
            "target_ref": target_ref,
            "target_state": target_state,
        }
    ]

    events.append({"type": "accepted", "reason": "target_resolved"})
    return AskPreflight(
        target_thread_id=resolved_target_id,
        target_ref=target_ref,
        target_state=target_state,
        route="ask",
        accepted=True,
        can_steer=False,
        not_sent_reason="",
        events=events,
    )


def probe_codex_cli(path: str | None = None) -> tuple[str, str]:
    cli_path = path or shutil.which("codex") or ""
    if not cli_path:
        return "", "not_found"
    try:
        completed = subprocess.run(
            [cli_path, "--version"],
            capture_output=True,
            text=True,
            timeout=5,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
    except PermissionError:
        return cli_path, "permission_denied"
    except Exception:  # noqa: BROAD_EXCEPT_OK
        return cli_path, "unavailable"
    return cli_path, "ok" if completed.returncode == 0 else "unavailable"


def get_runtime_status(*, bridge_module: DesktopDiscoveryBridge = bridge) -> HarnessRuntime:
    cli_path, cli_status = probe_codex_cli()
    desktop_status = "unknown"
    try:
        desktop_path, _desktop_source = bridge_module.discover_codex_desktop_executable()
        desktop_status = "available" if desktop_path is not None else "not_found"
    except Exception:  # noqa: BROAD_EXCEPT_OK
        desktop_status = "unavailable"
    return HarnessRuntime(
        version=HARNESS_VERSION,
        platform="windows-local",
        codex_cli_path=cli_path,
        codex_cli_status=cli_status,
        codex_desktop_status=desktop_status,
    )


def print_json(data: PrintableJsonData) -> None:
    if isinstance(data, DictExportable):
        data = data.to_dict()
    elif is_dataclass(data):
        data = cast(JsonObject, asdict(data))
    print(json.dumps(data, ensure_ascii=False, indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description="Windows-local Codex frontend harness status/preflight.")
    subparsers = parser.add_subparsers(dest="command", required=True)
    _ = subparsers.add_parser("runtime")
    preflight_parser = subparsers.add_parser("preflight")
    _ = preflight_parser.add_argument("--thread-id", default=None)
    args = HarnessArgs(command="", thread_id=None)
    _ = parser.parse_args(namespace=args)

    if args.command == "runtime":
        print_json(get_runtime_status())
        return 0
    if args.command == "preflight":
        print_json(preflight_ask(args.thread_id))
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
