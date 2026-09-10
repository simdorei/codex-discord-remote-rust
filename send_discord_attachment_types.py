from __future__ import annotations

import argparse
from dataclasses import dataclass
from typing import TypeAlias


JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
DiscordMessageResponse: TypeAlias = dict[str, JsonValue]


@dataclass(frozen=True, slots=True)
class AttachmentCliArgs:
    channel_id: str | None
    thread_ref: str | None
    work_thread: str | None
    content: str
    content_file: str | None
    files: list[str]


class AttachmentArgNamespace(argparse.Namespace):
    channel_id: str | None
    thread_ref: str | None
    work_thread: str | None
    content: str
    content_file: str | None
    files: list[str]

    def __init__(self) -> None:
        super().__init__()
        self.channel_id = None
        self.thread_ref = None
        self.work_thread = None
        self.content = ""
        self.content_file = None
        self.files = []


class MissingDiscordBotTokenError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("DISCORD_BOT_TOKEN is missing from environment or .env")


class DiscordSendFailedError(RuntimeError):
    status: int
    response_text: str

    def __init__(self, *, status: int, response_text: str) -> None:
        self.status = status
        self.response_text = response_text
        super().__init__(f"Discord send failed: HTTP {status}: {response_text}")


class AttachmentTargetError(RuntimeError):
    pass


class DiscordChannelAccessError(AttachmentTargetError):
    status: int
    response_text: str

    def __init__(self, *, status: int, response_text: str, message: str) -> None:
        self.status = status
        self.response_text = response_text
        super().__init__(message)
