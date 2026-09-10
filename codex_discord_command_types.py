from __future__ import annotations

from dataclasses import dataclass
from typing import TypeAlias


RawCommandValue: TypeAlias = str | int | None


@dataclass(frozen=True, slots=True)
class PrefixCommand:
    command: str
    arg: str


@dataclass(frozen=True, slots=True)
class PrefixBridgeAction:
    argv: list[str] | None
    title: str
    usage: str | None = None


@dataclass(frozen=True, slots=True)
class PrefixLimitAction:
    limit: int | None
    usage: str | None = None


@dataclass(frozen=True, slots=True)
class MirrorAction:
    subcommand: str | None
    limit: int | None = None
    usage: str | None = None
