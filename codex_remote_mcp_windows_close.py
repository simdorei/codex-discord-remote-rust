from __future__ import annotations

from typing import Final

from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_windows_platform_lifecycle import (
    OwnedLaunch,
    owned_launch_exit_code,
)
from codex_remote_mcp_windows_process_stop import stop_retained_process
from codex_remote_mcp_windows_windows import require_matching_active_window

CLOSE_TIMEOUT_SECONDS: Final = 2.0
KILL_TIMEOUT_SECONDS: Final = 5.0


def close_owned_window(
    identity: ComputerWindowIdentity,
    launch: OwnedLaunch,
) -> None:
    if owned_launch_exit_code(launch) is not None:
        return
    _ = require_matching_active_window(identity)
    if owned_launch_exit_code(launch) is not None:
        return
    stop_retained_process(
        launch.process,
        terminate_timeout_seconds=CLOSE_TIMEOUT_SECONDS,
        kill_timeout_seconds=KILL_TIMEOUT_SECONDS,
    )
