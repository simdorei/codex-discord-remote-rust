from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence
from typing import Literal, Protocol, TypeAlias

RelayMode = Literal["commentary", "failed", "final", "timeout", "transport_error"]


class RelayChannel(Protocol):
    pass


InteractiveNoticeOptions: TypeAlias = Sequence[tuple[str, str]]

SendChunksFunc = Callable[[RelayChannel, str], Awaitable[None]]
ParseInteractiveNoticeFunc = Callable[[str], tuple[str | None, str, InteractiveNoticeOptions]]
SendInteractivePromptFunc = Callable[[RelayChannel, str, str, str, str, InteractiveNoticeOptions], Awaitable[None]]
RegisterDiscordRelayFunc = Callable[[str | None], int]
IsDiscordRelayStaleFunc = Callable[[str | None, int], bool]
HadSteeringHandoffSinceFunc = Callable[[str | None, float], bool]
LogFunc = Callable[[str], None]
FormatLogTextLenFunc = Callable[[str], str]
