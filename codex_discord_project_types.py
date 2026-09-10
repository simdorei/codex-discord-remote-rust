from __future__ import annotations

from collections.abc import Callable, Mapping
from pathlib import Path
from typing import Protocol, TypeAlias

import codex_discord_project_channels as project_channels

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


class ProjectThread(Protocol):
    @property
    def cwd(self) -> str | None: ...


class BridgeProjectModule(Protocol):
    @property
    def GLOBAL_STATE_PATH(self) -> Path: ...

    def normalize_workspace_path(self, path: str) -> str: ...

    def strip_windows_extended_prefix(self, path: str) -> str: ...

    def get_thread_workspace_name(self, thread: ProjectThread) -> str: ...

    def load_json(self, path: Path) -> Mapping[str, JsonValue]: ...

    def choose_thread(self, thread_id: str, cwd: str | None) -> ProjectThread: ...


GetMirroredCodexThreadId = Callable[[int | None], str | None]
GetThreadCwd = Callable[[str | None], str | None]
GetMirrorProjectForChannel = Callable[[int | None], tuple[str, str] | None]
FindProjectlessNewChatCwd = Callable[[], str | None]
InitMirrorDb: TypeAlias = project_channels.InitMirrorDb
ProjectKeysMatch: TypeAlias = project_channels.ProjectKeysMatch
SqlRow: TypeAlias = project_channels.SqlRow
