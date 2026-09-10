import unittest
from dataclasses import dataclass

import codex_discord_prefix_archive_commands as prefix_archive


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel

    @classmethod
    def make(cls, *, channel_id: int = 222) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id))


class PrefixArchiveCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        exit_code: int = 0,
        output: str = "preview output",
    ) -> tuple[prefix_archive.PrefixArchiveCommandDeps, list[str], list[list[str]]]:
        sent: list[str] = []
        calls: list[list[str]] = []

        async def send_chunks(
            target: prefix_archive.ChannelLike,
            text: str,
            *,
            context: str = "send_chunks",
        ) -> int:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        def run_bridge_command(argv: list[str]) -> tuple[int, str]:
            calls.append(argv)
            return exit_code, output

        deps = prefix_archive.PrefixArchiveCommandDeps(
            send_chunks=send_chunks,
            run_bridge_command=run_bridge_command,
        )
        return deps, sent, calls

    async def test_dispatches_delete_archive_preview(self) -> None:
        deps, sent, calls = self.make_deps(output="archive preview")
        message = FakeMessage.make()

        handled = await prefix_archive.handle_prefix_archive_command(
            "delete_archive",
            "thread-1",
            message,
            deps=deps,
        )

        self.assertTrue(handled)
        self.assertEqual(calls, [["delete_archive", "thread-1"]])
        self.assertEqual(
            sent,
            [
                "send_chunks:Delete archive preview\n\n"
                + "archive preview\n\n"
                + "To actually delete it, run `!confirm_delete_archive <thread_id>`.",
            ],
        )

    async def test_preserves_usage_failure_empty_output_and_unhandled(self) -> None:
        deps, sent, calls = self.make_deps()
        message = FakeMessage.make()

        self.assertFalse(
            await prefix_archive.handle_prefix_archive_command(
                "confirm_delete_archive",
                "thread-1",
                message,
                deps=deps,
            )
        )
        self.assertEqual(sent, [])
        self.assertEqual(calls, [])

        self.assertTrue(await prefix_archive.handle_prefix_archive_command("delete_archive", "", message, deps=deps))
        self.assertEqual(sent[-1], "prefix_delete_archive_usage:Usage: !delete_archive <ref>")
        self.assertEqual(calls, [])

        deps, sent, calls = self.make_deps(exit_code=7, output="")
        self.assertTrue(
            await prefix_archive.handle_prefix_archive_command(
                "delete_archive",
                "thread-2",
                message,
                deps=deps,
            )
        )
        self.assertEqual(calls, [["delete_archive", "thread-2"]])
        self.assertEqual(
            sent,
            [
                "send_chunks:Delete archive failed (exit 7)\n\n"
                + "(no output)\n\n"
                + "To actually delete it, run `!confirm_delete_archive <thread_id>`.",
            ],
        )


if __name__ == "__main__":
    _ = unittest.main()
