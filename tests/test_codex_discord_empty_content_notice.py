from __future__ import annotations

import unittest

import codex_discord_empty_content_notice as empty_content_notice


class FakeEmptyContentChannel:
    def __init__(self, channel_id: int) -> None:
        self.id = channel_id


class FakeEmptyContentMessage:
    def __init__(self, channel: FakeEmptyContentChannel, *, non_text: bool = False) -> None:
        self.channel = channel
        self.non_text = non_text


class EmptyContentNoticeTests(unittest.TestCase):
    def test_empty_content_notice_updates_first_send_time(self) -> None:
        last_sent: dict[int, float] = {}

        self.assertTrue(
            empty_content_notice.should_send_empty_content_notice(
                333,
                last_sent=last_sent,
                cooldown_seconds=10.0,
                now=100.0,
            )
        )
        self.assertEqual(last_sent, {333: 100.0})

    def test_empty_content_notice_respects_cooldown(self) -> None:
        last_sent = {333: 100.0}

        self.assertFalse(
            empty_content_notice.should_send_empty_content_notice(
                333,
                last_sent=last_sent,
                cooldown_seconds=10.0,
                now=105.0,
            )
        )
        self.assertEqual(last_sent, {333: 100.0})

    def test_empty_content_notice_allows_after_cooldown(self) -> None:
        last_sent = {333: 100.0}

        self.assertTrue(
            empty_content_notice.should_send_empty_content_notice(
                333,
                last_sent=last_sent,
                cooldown_seconds=10.0,
                now=111.0,
            )
        )
        self.assertEqual(last_sent, {333: 111.0})

    def test_empty_content_notice_skips_missing_channel(self) -> None:
        last_sent: dict[int, float] = {}

        self.assertFalse(
            empty_content_notice.should_send_empty_content_notice(
                None,
                last_sent=last_sent,
                cooldown_seconds=10.0,
                now=100.0,
            )
        )
        self.assertEqual(last_sent, {})


class EmptyContentNoticeSendTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        last_sent: dict[int, float] | None = None,
        non_text: bool = False,
    ) -> tuple[
        empty_content_notice.EmptyContentNoticeDeps,
        list[tuple[empty_content_notice.EmptyContentChannel, str]],
        list[str],
        dict[int, float],
    ]:
        sent: list[tuple[empty_content_notice.EmptyContentChannel, str]] = []
        logs: list[str] = []
        seen = {} if last_sent is None else last_sent

        def has_non_text_payload(message: empty_content_notice.EmptyContentMessage) -> bool:
            _ = message
            return non_text

        async def send_chunks(channel: empty_content_notice.EmptyContentChannel, text: str) -> int:
            sent.append((channel, text))
            return 1

        deps = empty_content_notice.EmptyContentNoticeDeps(
            message_has_non_text_payload=has_non_text_payload,
            last_sent=seen,
            cooldown_seconds=10.0,
            send_chunks=send_chunks,
            log_line=logs.append,
        )
        return deps, sent, logs, seen

    async def test_sends_empty_content_notice_and_logs(self) -> None:
        channel = FakeEmptyContentChannel(333)
        message = FakeEmptyContentMessage(channel)
        deps, sent, logs, seen = self.make_deps()

        await empty_content_notice.maybe_send_empty_content_notice(message, deps=deps)

        self.assertEqual(sent, [(channel, empty_content_notice.EMPTY_CONTENT_NOTICE_TEXT)])
        self.assertEqual(list(seen), [333])
        self.assertEqual(logs, ["empty_content_notice_sent chat=333"])

    async def test_skips_non_text_payload_before_cooldown(self) -> None:
        message = FakeEmptyContentMessage(FakeEmptyContentChannel(333), non_text=True)
        deps, sent, logs, seen = self.make_deps(non_text=True)

        await empty_content_notice.maybe_send_empty_content_notice(message, deps=deps)

        self.assertEqual(sent, [])
        self.assertEqual(seen, {})
        self.assertEqual(logs, ["empty_content_notice_skipped reason=non_text_payload chat=333"])

    async def test_skips_recent_notice(self) -> None:
        message = FakeEmptyContentMessage(FakeEmptyContentChannel(333))
        deps, sent, logs, seen = self.make_deps(last_sent={333: 999999999.0})

        await empty_content_notice.maybe_send_empty_content_notice(message, deps=deps)

        self.assertEqual(sent, [])
        self.assertEqual(seen, {333: 999999999.0})
        self.assertEqual(logs, ["empty_content_notice_skipped reason=cooldown chat=333"])


if __name__ == "__main__":
    _ = unittest.main()
