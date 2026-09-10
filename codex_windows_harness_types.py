from __future__ import annotations

import argparse
from dataclasses import dataclass, field
from pathlib import Path
from typing import Protocol, TypeAlias, cast, runtime_checkable

from codex_thread_models import ThreadInfo

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


class ThreadRefBridge(Protocol):
    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str: ...


class ThreadLabelBridge(Protocol):
    def get_thread_label(self, thread: ThreadInfo) -> str: ...


class ThreadSnapshotBridge(ThreadLabelBridge, Protocol):
    pass


class TargetThreadBridge(ThreadRefBridge, ThreadLabelBridge, Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...


class DesktopDiscoveryBridge(Protocol):
    def discover_codex_desktop_executable(self) -> tuple[Path | None, str]: ...


@runtime_checkable
class DictExportable(Protocol):
    def to_dict(self) -> JsonObject: ...


class HarnessArgs(argparse.Namespace):
    command: str
    thread_id: str | None

    def __init__(self, *, command: str, thread_id: str | None) -> None:
        super().__init__()
        self.command = command
        self.thread_id = thread_id


@dataclass(frozen=True, slots=True)
class HarnessThread:
    id: str
    ref: str
    title: str
    cwd: str
    state: str
    updated_at: int


@dataclass(frozen=True, slots=True)
class HarnessRuntime:
    version: str
    platform: str
    codex_cli_path: str
    codex_cli_status: str
    codex_desktop_status: str


@dataclass(frozen=True, slots=True)
class AskPreflight:
    target_thread_id: str | None
    target_ref: str
    target_state: str
    route: str
    accepted: bool
    can_steer: bool
    not_sent_reason: str
    events: list[JsonObject] = field(default_factory=list)

    def to_dict(self) -> JsonObject:
        return cast(
            JsonObject,
            {
                "target_thread_id": self.target_thread_id,
                "target_ref": self.target_ref,
                "target_state": self.target_state,
                "route": self.route,
                "accepted": self.accepted,
                "can_steer": self.can_steer,
                "not_sent_reason": self.not_sent_reason,
                "events": self.events,
            },
        )


PrintableJsonData: TypeAlias = JsonValue | DictExportable | HarnessThread | HarnessRuntime | AskPreflight
