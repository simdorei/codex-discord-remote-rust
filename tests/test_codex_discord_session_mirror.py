from __future__ import annotations

from collections.abc import Mapping
from typing import cast
import unittest

from codex_app_server_transport_goal import ThreadGoalStatus
from codex_session_events import JsonEvent, JsonValue
import codex_discord_session_mirror as session_mirror
import codex_discord_session_mirror_item_append as item_append
import codex_discord_session_mirror_item_builders as item_builders


class SessionMirrorTextFormattingTests(unittest.TestCase):
    def test_final_answer_has_an_explicit_final_label(self) -> None:
        self.assertEqual(
            session_mirror.format_session_mirror_text(
                {"kind": "final", "text": "done"}
            ),
            "Final\n\ndone",
        )

    def test_blocked_goal_turn_completion_is_final(self) -> None:
        events: list[JsonEvent] = [
            {
                "timestamp": "1",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "blocked goal report",
                },
            }
        ]

        items = session_mirror.collect_session_mirror_items(
            "thread-1",
            events,
            seen_agent_messages={},
            seen_user_messages={},
            should_skip_discord_origin_prompt_func=lambda _thread_id, _text: False,
            build_interactive_notice_func=_build_interactive_notice,
            extract_message_text_func=_extract_message_text,
            recent_text_ttl_seconds=600.0,
            goal_status=ThreadGoalStatus.BLOCKED,
        )

        self.assertEqual([item["kind"] for item in items], ["final"])
        self.assertEqual(items[0]["phase"], "final_answer")


def _extract_message_text(payload: Mapping[str, JsonValue]) -> str:
    content = payload.get("content")
    if not isinstance(content, list):
        return str(payload.get("message") or "").strip()
    texts: list[str] = []
    for part in content:
        if isinstance(part, Mapping):
            part_mapping = cast(Mapping[str, JsonValue], part)
            texts.append(str(part_mapping.get("text") or ""))
    return "".join(texts).strip()


def _build_interactive_notice(payload: Mapping[str, JsonValue]) -> str | None:
    _ = payload
    return None


class SessionMirrorTargetTests(unittest.TestCase):
    def test_parse_session_mirror_target_accepts_valid_mapping(self) -> None:
        target = session_mirror.parse_session_mirror_target(
            {
                "codex_thread_id": 123,
                "discord_thread_id": "456",
            }
        )

        self.assertIsNotNone(target)
        assert target is not None
        self.assertEqual(target.codex_thread_id, "123")
        self.assertEqual(target.discord_thread_id, 456)

    def test_parse_session_mirror_target_rejects_missing_or_malformed_mapping(
        self,
    ) -> None:
        cases = [
            {},
            {"codex_thread_id": "", "discord_thread_id": "456"},
            {"codex_thread_id": "thread-1"},
            {"codex_thread_id": "thread-1", "discord_thread_id": "0"},
            {"codex_thread_id": "thread-1", "discord_thread_id": "not-int"},
        ]

        for candidate in cases:
            with self.subTest(candidate=candidate):
                self.assertIsNone(session_mirror.parse_session_mirror_target(candidate))


class SessionMirrorItemAppendTests(unittest.TestCase):
    def test_append_user_if_new_skips_discord_echo_and_remembers_text(self) -> None:
        ctx = _append_context(skip_texts={"from discord"})
        items: list[dict[str, str]] = []

        item_append.append_user_if_new(ctx, items, _event("1"), "from discord", "input")

        self.assertEqual(items, [])
        self.assertTrue(
            item_builders.has_recent_session_text(
                ctx.seen_user_messages,
                "from discord",
                ttl_seconds=600.0,
                make_text_digest_func=item_builders.make_text_digest,
            )
        )

    def test_append_agent_if_new_suppresses_duplicate_text(self) -> None:
        ctx = _append_context()
        items: list[dict[str, str]] = []

        item_append.append_agent_if_new(
            ctx, items, _event("1"), "working", kind="commentary", phase="commentary"
        )
        item_append.append_agent_if_new(
            ctx, items, _event("2"), "working", kind="commentary", phase="commentary"
        )

        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["kind"], "commentary")
        self.assertEqual(items[0]["role"], "assistant")
        self.assertEqual(items[0]["text"], "working")

    def test_has_terminal_assistant_item_only_matches_final_or_aborted_assistant(
        self,
    ) -> None:
        self.assertFalse(
            item_append.has_terminal_assistant_item(
                [{"role": "assistant", "kind": "commentary"}]
            )
        )
        self.assertTrue(
            item_append.has_terminal_assistant_item(
                [{"role": "assistant", "kind": "final"}]
            )
        )
        self.assertTrue(
            item_append.has_terminal_assistant_item(
                [{"role": "assistant", "kind": "aborted"}]
            )
        )


def _append_context(
    skip_texts: set[str] | None = None,
) -> item_append.CollectionContext:
    def should_skip_discord_origin_prompt(
        _codex_thread_id: str | None, text: str
    ) -> bool:
        return text in (skip_texts or set())

    return item_append.CollectionContext(
        codex_thread_id="thread-1",
        seen_agent_messages={},
        seen_user_messages={},
        should_skip_discord_origin_prompt=should_skip_discord_origin_prompt,
        build_interactive_notice=_build_interactive_notice,
        extract_message_text=_extract_message_text,
        recent_text_ttl_seconds=600.0,
        make_text_digest=item_builders.make_text_digest,
        goal_status=None,
    )


def _event(timestamp: str) -> JsonEvent:
    return {"timestamp": timestamp, "type": "event_msg", "payload": {}}


if __name__ == "__main__":
    _ = unittest.main()
