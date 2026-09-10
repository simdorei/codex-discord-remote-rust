import unittest
from dataclasses import dataclass

import codex_discord_prefix_mirror_commands as prefix_mirror
from codex_discord_session_mirror_detail import SessionMirrorDetailMode


@dataclass(frozen=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True)
class FakeMessage:
    channel: FakeChannel

    @classmethod
    def make(cls, channel_id: int = 222) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id))


class PrefixMirrorCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        bridge_output: str = "Discord bridge sync complete.",
        mirror_sync_output: str = "Mirror sync complete.",
        mirror_list_output: str = "Mirror list.",
        mirror_check_output: str = "Mirror check.",
        mirrored_thread_id: str | None = "thread-1",
    ) -> tuple[
        prefix_mirror.PrefixMirrorCommandDeps,
        list[str],
        list[tuple[str, object, int | None, int | None]],
        list[str],
    ]:
        sent: list[str] = []
        calls: list[tuple[str, object, int | None, int | None]] = []
        logs: list[str] = []
        detail_modes = {"thread-1": SessionMirrorDetailMode.SEND}

        async def send_chunks(target: object, text: str, *, context: str = "send_chunks") -> object:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        async def refresh(bot: object, *, limit: int | None = None) -> str:
            calls.append(("bridge", bot, limit, None))
            return bridge_output

        async def sync(bot: object, *, limit: int | None = None) -> str:
            calls.append(("mirror_sync", bot, limit, None))
            return mirror_sync_output

        def mirror_list(bot: object, limit: int | None = None, *, channel_id: int | None = None) -> str:
            calls.append(("mirror_list", bot, limit, channel_id))
            return mirror_list_output

        def mirror_check(bot: object, limit: int | None = None, *, channel_id: int | None = None) -> str:
            calls.append(("mirror_check", bot, limit, channel_id))
            return mirror_check_output

        def get_detail_mode(thread_id: str) -> SessionMirrorDetailMode:
            calls.append(("detail_get", thread_id, None, None))
            return detail_modes[thread_id]

        def set_detail_mode(
            thread_id: str,
            mode: SessionMirrorDetailMode,
        ) -> None:
            calls.append(("detail_set", thread_id, None, None))
            detail_modes[thread_id] = mode

        deps = prefix_mirror.PrefixMirrorCommandDeps(
            send_chunks=send_chunks,
            refresh_discord_bridge_session=refresh,
            sync_codex_mirror=sync,
            build_mirror_list=mirror_list,
            build_mirror_check=mirror_check,
            get_mirrored_codex_thread_id=lambda channel_id: (
                mirrored_thread_id if channel_id == 222 else None
            ),
            describe_mirrored_project_channel=lambda channel_id: (
                f"Select a mirrored Codex thread under channel {channel_id}."
            ),
            get_session_mirror_detail_mode=get_detail_mode,
            set_session_mirror_detail_mode=set_detail_mode,
            log_line=logs.append,
        )
        return deps, sent, calls, logs

    async def test_dispatches_happy_path_bridge_and_mirror_commands(self) -> None:
        deps, sent, calls, logs = self.make_deps()
        message = FakeMessage.make()
        fake_bot = object()

        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("bridge", "sync 17", message, fake_bot, deps=deps))
        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "sync", message, fake_bot, deps=deps))
        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "list 9", message, fake_bot, deps=deps))
        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "check 7", message, fake_bot, deps=deps))

        self.assertEqual(
            sent,
            [
                "prefix_bridge_sync_start:Discord bridge sync started.",
                "send_chunks:Discord bridge sync complete.",
                "prefix_mirror_sync_start:Mirror sync started.",
                "send_chunks:Mirror sync complete.",
                "send_chunks:Mirror list.",
                "send_chunks:Mirror check.",
            ],
        )
        self.assertEqual(
            calls,
            [
                ("bridge", fake_bot, 17, None),
                ("mirror_sync", fake_bot, None, None),
                ("mirror_list", fake_bot, 9, None),
                ("mirror_check", fake_bot, 7, None),
            ],
        )
        self.assertEqual(logs, [])

    async def test_preserves_usage_errors_failures_and_unhandled_commands(self) -> None:
        deps, sent, calls, logs = self.make_deps()
        message = FakeMessage.make()
        fake_bot = object()

        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("bridge", "bad 1", message, fake_bot, deps=deps))
        self.assertEqual(sent[-1], "prefix_bridge_sync_usage:Usage: !bridge sync [limit]")

        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "sync 1", message, fake_bot, deps=deps))
        self.assertEqual(sent[-1], "prefix_mirror_usage:Usage: !mirror sync")

        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "bad", message, fake_bot, deps=deps))
        self.assertEqual(sent[-1], "prefix_mirror_usage:Usage: !mirror sync | !mirror list [limit] | !mirror check [limit]")

        async def failing_refresh(bot: object, *, limit: int | None = None) -> str:
            _ = bot, limit
            raise RuntimeError("refresh failed")

        deps = prefix_mirror.PrefixMirrorCommandDeps(
            send_chunks=deps.send_chunks,
            refresh_discord_bridge_session=failing_refresh,
            sync_codex_mirror=deps.sync_codex_mirror,
            build_mirror_list=deps.build_mirror_list,
            build_mirror_check=deps.build_mirror_check,
            get_mirrored_codex_thread_id=deps.get_mirrored_codex_thread_id,
            describe_mirrored_project_channel=deps.describe_mirrored_project_channel,
            get_session_mirror_detail_mode=deps.get_session_mirror_detail_mode,
            set_session_mirror_detail_mode=deps.set_session_mirror_detail_mode,
            log_line=logs.append,
        )
        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("sync", "", message, fake_bot, deps=deps))
        self.assertIn("Discord bridge sync failed\n\nERROR: refresh failed", sent[-1])
        self.assertTrue(any(line.startswith("bridge_sync_failed\n") for line in logs))

        async def failing_sync(bot: object, *, limit: int | None = None) -> str:
            _ = bot, limit
            raise RuntimeError("sync failed")

        def failing_list(bot: object, limit: int | None = None, *, channel_id: int | None = None) -> str:
            _ = bot, limit, channel_id
            raise RuntimeError("list failed")

        def failing_check(bot: object, limit: int | None = None, *, channel_id: int | None = None) -> str:
            _ = bot, limit, channel_id
            raise RuntimeError("check failed")

        deps = prefix_mirror.PrefixMirrorCommandDeps(
            send_chunks=deps.send_chunks,
            refresh_discord_bridge_session=deps.refresh_discord_bridge_session,
            sync_codex_mirror=failing_sync,
            build_mirror_list=failing_list,
            build_mirror_check=failing_check,
            get_mirrored_codex_thread_id=deps.get_mirrored_codex_thread_id,
            describe_mirrored_project_channel=deps.describe_mirrored_project_channel,
            get_session_mirror_detail_mode=deps.get_session_mirror_detail_mode,
            set_session_mirror_detail_mode=deps.set_session_mirror_detail_mode,
            log_line=logs.append,
        )
        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "sync", message, fake_bot, deps=deps))
        self.assertIn("Mirror sync failed\n\nERROR: sync failed", sent[-1])
        self.assertTrue(any(line.startswith("mirror_sync_failed\n") for line in logs))

        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "list 3", message, fake_bot, deps=deps))
        self.assertIn("Mirror list failed\n\nERROR: list failed", sent[-1])
        self.assertTrue(any(line.startswith("mirror_list_failed\n") for line in logs))

        self.assertTrue(await prefix_mirror.handle_prefix_mirror_command("mirror", "check 3", message, fake_bot, deps=deps))
        self.assertIn("Mirror check failed\n\nERROR: check failed", sent[-1])
        self.assertTrue(any(line.startswith("mirror_check_failed\n") for line in logs))

        self.assertFalse(await prefix_mirror.handle_prefix_mirror_command("where", "", message, fake_bot, deps=deps))
        self.assertEqual(calls, [])

    async def test_detail_shows_and_changes_mode_for_mapped_thread(self) -> None:
        deps, sent, calls, logs = self.make_deps()
        message = FakeMessage.make()
        fake_bot = object()

        self.assertTrue(
            await prefix_mirror.handle_prefix_mirror_command(
                "detail", "", message, fake_bot, deps=deps
            )
        )
        self.assertIn("send", sent[-1])

        self.assertTrue(
            await prefix_mirror.handle_prefix_mirror_command(
                "detail", "all", message, fake_bot, deps=deps
            )
        )
        self.assertIn("all", sent[-1])

        self.assertTrue(
            await prefix_mirror.handle_prefix_mirror_command(
                "detail", "", message, fake_bot, deps=deps
            )
        )
        self.assertIn("all", sent[-1])
        self.assertEqual(
            [call[0] for call in calls],
            ["detail_get", "detail_set", "detail_get"],
        )
        self.assertEqual(logs, [])

    async def test_detail_rejects_invalid_mode_and_project_channel(self) -> None:
        deps, sent, calls, _logs = self.make_deps()
        message = FakeMessage.make()

        self.assertTrue(
            await prefix_mirror.handle_prefix_mirror_command(
                "detail", "verbose", message, object(), deps=deps
            )
        )
        self.assertEqual(
            sent[-1],
            "prefix_detail_usage:Usage: !detail | !detail send | !detail all",
        )
        self.assertEqual(calls, [])

        unmapped_deps, unmapped_sent, unmapped_calls, _logs = self.make_deps(
            mirrored_thread_id=None
        )
        self.assertTrue(
            await prefix_mirror.handle_prefix_mirror_command(
                "detail", "all", message, object(), deps=unmapped_deps
            )
        )
        self.assertIn("Select a mirrored Codex thread", unmapped_sent[-1])
        self.assertEqual(unmapped_calls, [])
