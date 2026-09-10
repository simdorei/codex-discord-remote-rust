from __future__ import annotations

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_files import ProjectFileError
from codex_remote_mcp_terminal_engine import TerminalExecutionError


def project_error_code(exc: ProjectFileError) -> str:
    if isinstance(exc, ComputerControlError):
        return "computer_control"
    if isinstance(exc, TerminalExecutionError):
        return "terminal_execution"
    return type(exc).__name__.removesuffix("Error").casefold()


__all__ = ["project_error_code"]
