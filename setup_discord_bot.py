from __future__ import annotations

import argparse
import getpass
import json
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from types import TracebackType
from typing import Final, Protocol


DISCORD_APPLICATION_ME_URL: Final[str] = "https://discord.com/api/v10/applications/@me"
DEFAULT_BOT_ID: Final[str] = "123456789012345678"
DEFAULT_PERMISSION_BITS: Final[tuple[int, ...]] = (
    64,
    1024,
    2048,
    16384,
    32768,
    65536,
    2147483648,
    17179869184,
    34359738368,
    274877906944,
)


class SetupDiscordBotError(RuntimeError):
    pass


class DiscordTokenCheckError(SetupDiscordBotError):
    pass


class EnvFileValueError(SetupDiscordBotError):
    pass


class DiscordChannelIdError(SetupDiscordBotError):
    pass


@dataclass(frozen=True, slots=True)
class DiscordApplication:
    application_id: str
    name: str


class UrlResponse(Protocol):
    def __enter__(self) -> UrlResponse: ...

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool | None: ...

    def read(self) -> bytes: ...


class UrlOpen(Protocol):
    def __call__(self, request: urllib.request.Request, /, *, timeout: float) -> UrlResponse: ...


def get_default_bot_permissions() -> int:
    permissions = 0
    for bit in DEFAULT_PERMISSION_BITS:
        permissions |= bit
    return permissions


def new_discord_bot_invite_url(client_id: str, permissions: int) -> str:
    scope = urllib.parse.quote("bot applications.commands")
    return f"https://discord.com/oauth2/authorize?client_id={client_id}&scope={scope}&permissions={permissions}"


def parse_discord_application(payload: bytes) -> DiscordApplication:
    data = json.loads(payload.decode("utf-8"))
    if not isinstance(data, dict):
        raise DiscordTokenCheckError("Discord did not return an application object.")

    application_id = data.get("id")
    if not isinstance(application_id, str) or not application_id.strip():
        raise DiscordTokenCheckError("Discord did not return an application id.")

    name = data.get("name")
    display_name = name.strip() if isinstance(name, str) and name.strip() else "Discord application"
    return DiscordApplication(application_id=application_id.strip(), name=display_name)


def fetch_discord_application(bot_token: str, *, urlopen: UrlOpen = urllib.request.urlopen) -> DiscordApplication:
    request = urllib.request.Request(
        DISCORD_APPLICATION_ME_URL,
        headers={
            "Authorization": f"Bot {bot_token}",
            "User-Agent": "DiscordBot (https://github.com/simdorei/codex-discord-remote, 1.0)",
        },
        method="GET",
    )
    try:
        with urlopen(request, timeout=15.0) as response:
            return parse_discord_application(response.read())
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace").strip()
        suffix = f" Details: {detail}" if detail else ""
        raise DiscordTokenCheckError(
            f"Discord bot token check failed with HTTP {exc.code}.{suffix}"
        ) from exc
    except urllib.error.URLError as exc:
        raise DiscordTokenCheckError(f"Discord bot token check failed. Details: {exc.reason}") from exc


def set_env_file_value(path: Path, name: str, value: str) -> None:
    if "\n" in value or "\r" in value:
        raise EnvFileValueError(f"{name} cannot contain a newline.")

    newline = "\n"
    lines: list[str] = []
    if path.exists():
        text = path.read_text(encoding="utf-8")
        newline = "\r\n" if "\r\n" in text else "\n"
        lines = text.splitlines()

    found = False
    for index, raw_line in enumerate(lines):
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("#") or "=" not in raw_line:
            continue
        key, _, _ = raw_line.partition("=")
        if key.strip() == name:
            lines[index] = f"{name}={value}"
            found = True
            break

    if not found:
        lines.append(f"{name}={value}")

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(newline.join(lines) + newline, encoding="utf-8")


def get_env_file_value(path: Path, name: str) -> str:
    if not path.exists():
        return ""
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("#") or "=" not in raw_line:
            continue
        key, _, value = raw_line.partition("=")
        if key.strip() == name:
            return value.strip()
    return ""


def parse_discord_channel_id(raw: str) -> str:
    channel_id = raw.strip()
    if not channel_id:
        return ""
    if not channel_id.isdecimal():
        raise DiscordChannelIdError("Discord general channel ID must contain digits only.")
    return channel_id


def add_csv_value(existing: str, value: str) -> str:
    items = [item.strip() for item in existing.split(",") if item.strip()]
    if value not in items:
        items.append(value)
    return ",".join(items)


def configure_general_channel_id(env_path: Path, channel_id: str) -> None:
    parsed_channel_id = parse_discord_channel_id(channel_id)
    if not parsed_channel_id:
        return
    allowed = get_env_file_value(env_path, "DISCORD_ALLOWED_CHANNEL_IDS")
    set_env_file_value(
        env_path,
        "DISCORD_ALLOWED_CHANNEL_IDS",
        add_csv_value(allowed, parsed_channel_id),
    )
    if not get_env_file_value(env_path, "DISCORD_STARTUP_CHANNEL_ID"):
        set_env_file_value(env_path, "DISCORD_STARTUP_CHANNEL_ID", parsed_channel_id)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Configure the Discord bot token and print the invite link.")
    parser.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--bot-id", default=DEFAULT_BOT_ID)
    return parser


def run(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    repo_root = args.repo_root.resolve()
    env_path = repo_root / ".env"
    permissions = get_default_bot_permissions()

    if args.dry_run:
        print("Dry run: no token was requested and .env was not changed.")
        print(f"Would save DISCORD_BOT_TOKEN to: {env_path}")
        print("Would ask for the Discord general channel ID for !commands.")
        print("Invite link:")
        print(new_discord_bot_invite_url(str(args.bot_id), permissions))
        return 0

    print("Paste the Discord bot token. Input is hidden.")
    token = getpass.getpass("Discord bot token: ").strip()
    if not token:
        raise SetupDiscordBotError("Discord bot token is empty.")

    application = fetch_discord_application(token)
    set_env_file_value(env_path, "DISCORD_BOT_TOKEN", token)

    print("Paste the Discord general text channel ID for !commands. Leave blank to fill it in later.")
    general_channel_id = input("Discord general channel ID: ").strip()
    if general_channel_id:
        configure_general_channel_id(env_path, general_channel_id)

    print(f"Discord bot token saved to: {env_path}")
    if general_channel_id:
        print(
            "General channel ID saved to DISCORD_ALLOWED_CHANNEL_IDS"
            + " and DISCORD_STARTUP_CHANNEL_ID when it was empty."
        )
    print(f"Application: {application.name} ({application.application_id})")
    print("Invite link:")
    print(new_discord_bot_invite_url(application.application_id, permissions))
    print("Open the invite link, choose your Discord server, and authorize the bot.")
    print("The invite link adds the bot to a server. Channel access still depends on Discord channel permissions and the channel IDs in .env.")
    print("Next: copy any remaining server/channel IDs into .env, restart Codex, then run the platform launcher.")
    return 0


def main() -> int:
    try:
        return run()
    except SetupDiscordBotError as exc:
        print(f"ERROR: {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
