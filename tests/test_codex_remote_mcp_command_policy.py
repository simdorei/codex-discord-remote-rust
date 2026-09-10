from __future__ import annotations

from typing import cast

import pytest

from codex_remote_mcp_command_policy import requires_execution_lock
from simdorei_mcp_common.messages import GatewayCommand


def test_unknown_runtime_command_fails_closed() -> None:
    command = cast(GatewayCommand, object())

    with pytest.raises(AssertionError):
        _ = requires_execution_lock(command)
