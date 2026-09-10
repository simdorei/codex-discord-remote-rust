from __future__ import annotations

from contextlib import contextmanager
from collections.abc import Generator

import pytest

from codex_remote_mcp_terminal_engine import TerminalExecutionError


@contextmanager
def _passthrough_context() -> Generator[None]:
    yield


def test_project_error_can_propagate_through_a_context_manager() -> None:
    with pytest.raises(TerminalExecutionError, match="synthetic failure"):
        with _passthrough_context():
            raise TerminalExecutionError("synthetic failure")
