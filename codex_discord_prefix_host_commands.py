from __future__ import annotations

import subprocess
import traceback
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Final, Protocol

HOST_REBOOT_COMMANDS: Final[frozenset[str]] = frozenset({"reset_pc", "reboot_pc", "reset_computer"})
REBOOT_DELAY_SECONDS: Final[int] = 5
REBOOT_ALLOWLIST_REQUIRED_MESSAGE: Final[str] = (
    "Host reboot refused. DISCORD_ALLOWED_USER_IDS must contain at least one allowed Discord user ID."
)


class ChannelLike(Protocol):
    @property
    def id(self) -> int:
        ...


class MessageLike(Protocol):
    @property
    def channel(self) -> ChannelLike:
        ...


class SendChunksFunc(Protocol):
    def __call__(self, target: ChannelLike, text: str, *, context: str = "send_chunks") -> Awaitable[int]:
        ...


class HostRebootFunc(Protocol):
    def __call__(self, *, delay_seconds: int) -> None:
        ...


@dataclass(frozen=True, slots=True)
class PrefixHostCommandDeps:
    send_chunks: SendChunksFunc
    host_commands_enabled: Callable[[], bool]
    host_reboot_allowed_user_ids_configured: Callable[[], bool]
    request_host_reboot: HostRebootFunc
    log_line: Callable[[str], None]


def request_windows_reboot(*, delay_seconds: int = REBOOT_DELAY_SECONDS) -> None:
    _ = subprocess.Popen(
        [
            "shutdown.exe",
            "/r",
            "/t",
            str(delay_seconds),
            "/c",
            "Codex Discord reset_pc requested",
        ],
        close_fds=True,
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )


async def handle_prefix_host_command(
    command: str,
    arg: str,
    message: MessageLike,
    *,
    deps: PrefixHostCommandDeps,
) -> bool:
    if command not in HOST_REBOOT_COMMANDS:
        return False
    if not deps.host_commands_enabled():
        _ = await deps.send_chunks(
            message.channel,
            "Host commands are disabled. Set DISCORD_ENABLE_HOST_COMMANDS=1 to enable them.",
            context="prefix_host_disabled",
        )
        return True
    if arg.strip().lower() != "confirm":
        _ = await deps.send_chunks(message.channel, "Usage: !reset_pc confirm", context="prefix_reset_pc_usage")
        return True
    if not deps.host_reboot_allowed_user_ids_configured():
        _ = await deps.send_chunks(
            message.channel,
            REBOOT_ALLOWLIST_REQUIRED_MESSAGE,
            context="prefix_reset_pc_allowlist_required",
        )
        return True
    try:
        deps.request_host_reboot(delay_seconds=REBOOT_DELAY_SECONDS)
    except OSError as exc:
        deps.log_line("host_reboot_failed\n" + traceback.format_exc())
        _ = await deps.send_chunks(message.channel, f"PC reset failed\n\nERROR: {exc}", context="prefix_reset_pc_failed")
        return True
    _ = await deps.send_chunks(
        message.channel,
        f"PC reset requested. Windows will reboot in {REBOOT_DELAY_SECONDS} seconds.",
        context="prefix_reset_pc_requested",
    )
    return True
