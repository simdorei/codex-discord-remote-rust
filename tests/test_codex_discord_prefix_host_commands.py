from __future__ import annotations

import os
import unittest
from dataclasses import dataclass, field
from unittest.mock import patch

import codex_discord_prefix_host_commands as host_commands


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel = FakeChannel()


@dataclass(frozen=True, slots=True)
class HostCommandFixture:
    sent: list[str] = field(default_factory=list)
    reboot_calls: list[int] = field(default_factory=list)
    logs: list[str] = field(default_factory=list)
    reboot_error: OSError | None = None
    host_commands_enabled: bool = True
    host_reboot_allowed_user_ids_configured: bool = True

    async def send_chunks(
        self,
        target: host_commands.ChannelLike,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> int:
        self.sent.append(f"{target.id}:{context}:{text}")
        return len(text)

    def request_host_reboot(self, *, delay_seconds: int) -> None:
        self.reboot_calls.append(delay_seconds)
        if self.reboot_error is not None:
            raise self.reboot_error

    def deps(self) -> host_commands.PrefixHostCommandDeps:
        return host_commands.PrefixHostCommandDeps(
            send_chunks=self.send_chunks,
            host_commands_enabled=lambda: self.host_commands_enabled,
            host_reboot_allowed_user_ids_configured=lambda: self.host_reboot_allowed_user_ids_configured,
            request_host_reboot=self.request_host_reboot,
            log_line=self.logs.append,
        )


class PrefixHostCommandTests(unittest.IsolatedAsyncioTestCase):
    async def test_reset_pc_is_disabled_by_default_gate(self) -> None:
        fixture = HostCommandFixture(host_commands_enabled=False)

        handled = await host_commands.handle_prefix_host_command(
            "reset_pc",
            "confirm",
            FakeMessage(),
            deps=fixture.deps(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.reboot_calls, [])
        disabled_message = "Host commands are disabled. Set DISCORD_ENABLE_HOST_COMMANDS=1 to enable them."
        self.assertEqual(
            fixture.sent,
            [f"222:prefix_host_disabled:{disabled_message}"],
        )

    async def test_reset_pc_requires_confirm(self) -> None:
        fixture = HostCommandFixture()

        handled = await host_commands.handle_prefix_host_command(
            "reset_pc",
            "",
            FakeMessage(),
            deps=fixture.deps(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.reboot_calls, [])
        self.assertEqual(fixture.sent, ["222:prefix_reset_pc_usage:Usage: !reset_pc confirm"])

    async def test_reset_pc_confirm_requests_host_reboot(self) -> None:
        fixture = HostCommandFixture()

        handled = await host_commands.handle_prefix_host_command(
            "reset_pc",
            "confirm",
            FakeMessage(),
            deps=fixture.deps(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.reboot_calls, [5])
        self.assertEqual(
            fixture.sent,
            ["222:prefix_reset_pc_requested:PC reset requested. Windows will reboot in 5 seconds."],
        )

    async def test_reset_pc_confirm_refuses_when_injected_allowed_user_ids_are_empty(self) -> None:
        fixture = HostCommandFixture(
            host_commands_enabled=True,
            host_reboot_allowed_user_ids_configured=False,
        )

        with patch.dict(
            os.environ,
            {"DISCORD_ENABLE_HOST_COMMANDS": "0", "DISCORD_ALLOWED_USER_IDS": ""},
            clear=True,
        ):
            handled = await host_commands.handle_prefix_host_command(
                "reset_pc",
                "confirm",
                FakeMessage(),
                deps=fixture.deps(),
            )

        self.assertTrue(handled)
        self.assertEqual(fixture.reboot_calls, [])
        self.assertEqual(len(fixture.sent), 1)
        self.assertIn("DISCORD_ALLOWED_USER_IDS", fixture.sent[0])

    async def test_reset_pc_reports_reboot_failure(self) -> None:
        fixture = HostCommandFixture(reboot_error=OSError("shutdown denied"))

        handled = await host_commands.handle_prefix_host_command(
            "reboot_pc",
            "confirm",
            FakeMessage(),
            deps=fixture.deps(),
        )

        self.assertTrue(handled)
        self.assertEqual(fixture.reboot_calls, [5])
        self.assertEqual(fixture.sent, ["222:prefix_reset_pc_failed:PC reset failed\n\nERROR: shutdown denied"])
        self.assertTrue(any(line.startswith("host_reboot_failed\n") for line in fixture.logs))

    async def test_unrelated_command_is_not_handled(self) -> None:
        fixture = HostCommandFixture()

        handled = await host_commands.handle_prefix_host_command(
            "mirror",
            "sync",
            FakeMessage(),
            deps=fixture.deps(),
        )

        self.assertFalse(handled)
        self.assertEqual(fixture.sent, [])
        self.assertEqual(fixture.reboot_calls, [])

    def test_prefix_host_command_deps_is_slotted(self) -> None:
        deps = HostCommandFixture().deps()

        self.assertFalse(hasattr(deps, "__dict__"))


if __name__ == "__main__":
    _ = unittest.main()
