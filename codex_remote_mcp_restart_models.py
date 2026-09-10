from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, override

from simdorei_mcp_common.messages import ProjectUpsert


@dataclass(frozen=True, slots=True)
class RestartProject:
    project: ProjectUpsert
    root: Path


@dataclass(frozen=True, slots=True)
class RestartHandoffError(Exception):
    reason: str

    @override
    def __str__(self) -> str:
        return self.reason


class HandoffProtector(Protocol):
    def protect(self, payload: bytes) -> bytes: ...

    def unprotect(self, payload: bytes) -> bytes: ...


__all__ = ["HandoffProtector", "RestartHandoffError", "RestartProject"]
