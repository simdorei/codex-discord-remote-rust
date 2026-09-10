from __future__ import annotations

from typing import cast

import pytest

import codex_remote_mcp_computer_execute as computer_execute
from simdorei_mcp_common.operation_requests import ComputerOperation


def test_unknown_runtime_operation_fails_before_controller_access() -> None:
    request = cast(ComputerOperation, object())
    controller = cast(computer_execute.ComputerControllerLike, object())

    with pytest.raises(AssertionError):
        _ = computer_execute.execute_running_operation(request, controller)
