from __future__ import annotations

from codex_remote_mcp_files import ProjectFileError


class ComputerControlError(ProjectFileError):
    """Raised when a remote desktop action is unavailable or unsafe."""

    def __init__(self, reason: str) -> None:
        super().__init__("computer", reason)
