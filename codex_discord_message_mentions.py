from __future__ import annotations

import re
from collections.abc import Iterable
from typing import Final, Protocol, assert_never, cast, TypeAlias
ModuleValue: TypeAlias = object

DiscordIdValue = int | str | bytes | bytearray | None
DISCORD_USER_MENTION_RE: Final = re.compile(r"<@!?(\d+)>")


class MentionUserLike(Protocol):
    @property
    def id(self) -> DiscordIdValue: ...

    @property
    def bot(self) -> bool: ...


class MessageAuthorLike(Protocol):
    @property
    def bot(self) -> bool: ...


class MessageWithMentions(Protocol):
    @property
    def raw_mentions(self) -> ModuleValue | None: ...

    @property
    def mentions(self) -> ModuleValue | None: ...


class BotMessageWithMentions(MessageWithMentions, Protocol):
    @property
    def author(self) -> ModuleValue | None: ...


class DiscordClientWithMentions(Protocol):
    @property
    def plain_ask_mention_user_ids(self) -> Iterable[DiscordIdValue] | None: ...

    @property
    def user(self) -> MentionUserLike | None: ...


def _iter_id_values(value: Iterable[DiscordIdValue] | None) -> Iterable[DiscordIdValue]:
    if value is None or isinstance(value, str | bytes | bytearray):
        return ()
    return value


def _iter_mention_users(value: Iterable[MentionUserLike] | None) -> Iterable[MentionUserLike]:
    if value is None:
        return ()
    return value


def _coerce_discord_id(value: DiscordIdValue) -> int | None:
    raw: str
    match value:
        case None:
            return None
        case int() as raw_id:
            return raw_id
        case str() as raw:
            pass
        case bytes() | bytearray() as raw_bytes:
            try:
                raw = bytes(raw_bytes).decode("ascii")
            except UnicodeDecodeError:
                return None
        case _:
            assert_never(value)
    try:
        return int(raw)
    except ValueError:
        return None


def strip_required_plain_ask_mentions(
    content: str,
    required_user_ids: set[int],
) -> tuple[str, bool]:
    if not required_user_ids:
        return content, True
    required_id_text = {str(user_id) for user_id in required_user_ids}
    matched = False

    def replace_mention(match: re.Match[str]) -> str:
        nonlocal matched
        if match.group(1) in required_id_text:
            matched = True
            return " "
        return match.group(0)

    stripped = DISCORD_USER_MENTION_RE.sub(replace_mention, content)
    stripped = re.sub(r"[ \t]{2,}", " ", stripped).strip()
    return stripped, matched


def get_discord_message_mention_ids(message: MessageWithMentions) -> set[int]:
    mention_ids: set[int] = set()
    raw_mentions = cast(Iterable[DiscordIdValue] | None, getattr(message, "raw_mentions", None))
    for raw_id in _iter_id_values(raw_mentions):
        mention_id = _coerce_discord_id(raw_id)
        if mention_id is not None:
            mention_ids.add(mention_id)
    raw_mentioned_users = cast(Iterable[MentionUserLike] | None, getattr(message, "mentions", None))
    for user in _iter_mention_users(raw_mentioned_users):
        user_id = cast(DiscordIdValue, getattr(user, "id", None))
        mention_id = _coerce_discord_id(user_id)
        if mention_id is not None:
            mention_ids.add(mention_id)
    return mention_ids


def message_mentions_required_plain_ask_user(
    message: MessageWithMentions,
    required_user_ids: set[int],
) -> bool:
    return bool(required_user_ids.intersection(get_discord_message_mention_ids(message)))


def get_bridge_mention_user_ids(discord_client: DiscordClientWithMentions) -> set[int]:
    mention_user_ids: set[int] = set()
    raw_user_ids = cast(
        Iterable[DiscordIdValue] | None,
        getattr(discord_client, "plain_ask_mention_user_ids", None),
    )
    for user_id in _iter_id_values(raw_user_ids):
        mention_id = _coerce_discord_id(user_id)
        if mention_id is not None:
            mention_user_ids.add(mention_id)
    self_user = cast(MentionUserLike | None, getattr(discord_client, "user", None))
    self_user_id = None if self_user is None else cast(DiscordIdValue, getattr(self_user, "id", None))
    mention_id = _coerce_discord_id(self_user_id)
    if mention_id is not None:
        mention_user_ids.add(mention_id)
    return mention_user_ids


def message_mentions_bridge_user(
    message: MessageWithMentions,
    discord_client: DiscordClientWithMentions,
) -> bool:
    return message_mentions_required_plain_ask_user(
        message,
        get_bridge_mention_user_ids(discord_client),
    )


def is_bot_authored_bridge_mention(
    message: BotMessageWithMentions,
    discord_client: DiscordClientWithMentions,
) -> bool:
    author = cast(MessageAuthorLike | None, getattr(message, "author", None))
    if author is None or not bool(getattr(author, "bot", False)):
        return False
    return message_mentions_bridge_user(message, discord_client)


def message_mentions_other_bot(
    message: MessageWithMentions,
    required_user_ids: set[int],
) -> bool:
    raw_mentioned_users = cast(Iterable[MentionUserLike] | None, getattr(message, "mentions", None))
    for user in _iter_mention_users(raw_mentioned_users):
        user_id = cast(DiscordIdValue, getattr(user, "id", None))
        normalized_user_id = _coerce_discord_id(user_id)
        if normalized_user_id is None:
            continue
        if normalized_user_id in required_user_ids:
            continue
        if bool(getattr(user, "bot", False)):
            return True
    return False
