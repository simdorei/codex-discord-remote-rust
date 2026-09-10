import unittest
from dataclasses import dataclass

import codex_discord_prefix_queue_commands as prefix_queue


class QueueFailureError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int = 333


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel
    author: FakeAuthor

    @classmethod
    def make(cls, *, channel_id: int = 222, author_id: int = 333) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id), author=FakeAuthor(author_id))


class PrefixQueueCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        response: str = "retracted",
        fail: bool = False,
    ) -> tuple[
        prefix_queue.PrefixQueueCommandDeps,
        list[str],
        list[tuple[int | None, int | None, str | None]],
        list[str],
    ]:
        sent: list[str] = []
        calls: list[tuple[int | None, int | None, str | None]] = []
        logs: list[str] = []

        async def send_chunks(target: prefix_queue.ChannelLike, text: str, *, context: str = "send_chunks") -> int:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        async def retract_queued_ask_for_request(
            *,
            channel_id: int | None,
            user_id: int | None,
            ref: str | None,
        ) -> tuple[str, prefix_queue.QueueRetractResult]:
            calls.append((channel_id, user_id, ref))
            if fail:
                raise QueueFailureError("queue failed")
            return response, {"removed": 1}

        deps = prefix_queue.PrefixQueueCommandDeps(
            send_chunks=send_chunks,
            retract_queued_ask_for_request=retract_queued_ask_for_request,
            log_line=logs.append,
        )
        return deps, sent, calls, logs

    async def test_dispatches_retract_and_unqueue_with_ref_forwarding(self) -> None:
        deps, sent, calls, logs = self.make_deps(response="removed")
        message = FakeMessage.make(channel_id=222, author_id=333)

        self.assertTrue(await prefix_queue.handle_prefix_queue_command("retract", "", message, deps=deps))
        self.assertTrue(await prefix_queue.handle_prefix_queue_command("unqueue", "thread-7", message, deps=deps))

        self.assertEqual(calls, [(222, 333, None), (222, 333, "thread-7")])
        self.assertEqual(sent, ["send_chunks:removed", "send_chunks:removed"])
        self.assertEqual(logs, [])

    async def test_preserves_failure_logging_and_unhandled_fallthrough(self) -> None:
        deps, sent, calls, logs = self.make_deps(fail=True)
        message = FakeMessage.make()

        self.assertFalse(await prefix_queue.handle_prefix_queue_command("where", "", message, deps=deps))
        self.assertEqual(sent, [])
        self.assertEqual(calls, [])

        self.assertTrue(await prefix_queue.handle_prefix_queue_command("unqueue", "selected", message, deps=deps))
        self.assertEqual(calls, [(222, 333, "selected")])
        self.assertEqual(sent, ["send_chunks:Queue retract failed\n\nERROR: queue failed"])
        self.assertTrue(any(line.startswith("queue_retract_failed\n") for line in logs))


if __name__ == "__main__":
    _ = unittest.main()
