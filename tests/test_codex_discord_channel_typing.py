from __future__ import annotations

import unittest
from types import TracebackType

import codex_discord_channel_typing as channel_typing


class TypingManagerError(RuntimeError):
    pass


class BodyError(RuntimeError):
    pass


class NoTypingTarget:
    pass


class RecordingTypingManager:
    def __init__(self, *, fail_enter: bool = False, fail_exit: bool = False) -> None:
        self.entered: bool = False
        self.exited: bool = False
        self.fail_enter: bool = fail_enter
        self.fail_exit: bool = fail_exit

    async def __aenter__(self) -> None:
        self.entered = True
        if self.fail_enter:
            raise TypingManagerError("enter failed")

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.exited = True
        if self.fail_exit:
            raise TypingManagerError("exit failed")


class TypingTarget:
    def __init__(self, manager: RecordingTypingManager) -> None:
        self.manager: RecordingTypingManager = manager

    def typing(self) -> RecordingTypingManager:
        return self.manager


class ChannelTypingTests(unittest.IsolatedAsyncioTestCase):
    async def test_channel_typing_noops_without_typing_factory(self) -> None:
        logs: list[str] = []

        async with channel_typing.channel_typing(NoTypingTarget(), context="unit", log_func=logs.append):
            pass

        self.assertEqual(logs, [])

    async def test_channel_typing_enters_and_exits_manager(self) -> None:
        logs: list[str] = []
        manager = RecordingTypingManager()

        async with channel_typing.channel_typing(TypingTarget(manager), context="unit", log_func=logs.append):
            self.assertTrue(manager.entered)

        self.assertTrue(manager.exited)
        self.assertEqual(logs, [])

    async def test_channel_typing_logs_start_failure(self) -> None:
        logs: list[str] = []
        manager = RecordingTypingManager(fail_enter=True)

        async with channel_typing.channel_typing(TypingTarget(manager), context="unit", log_func=logs.append):
            pass

        self.assertEqual(logs, ["typing_start_failed context=unit error_type=TypingManagerError"])

    async def test_channel_typing_can_raise_start_failure_after_logging(self) -> None:
        logs: list[str] = []
        manager = RecordingTypingManager(fail_enter=True)

        with self.assertRaisesRegex(TypingManagerError, "enter failed"):
            async with channel_typing.channel_typing(
                TypingTarget(manager),
                context="unit",
                log_func=logs.append,
                raise_start_error=True,
            ):
                pass

        self.assertEqual(logs, ["typing_start_failed context=unit error_type=TypingManagerError"])

    async def test_channel_typing_logs_stop_failure_and_preserves_body_exception(self) -> None:
        logs: list[str] = []
        manager = RecordingTypingManager(fail_exit=True)

        with self.assertRaisesRegex(BodyError, "body failed"):
            async with channel_typing.channel_typing(TypingTarget(manager), context="unit", log_func=logs.append):
                raise BodyError("body failed")

        self.assertEqual(logs, ["typing_stop_failed context=unit error_type=TypingManagerError"])
