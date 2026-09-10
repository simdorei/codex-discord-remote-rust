from __future__ import annotations

import ctypes
import unittest
from unittest.mock import patch

import codex_desktop_bridge_windows_input as windows_input


class WindowsInputClipboardErrorTests(unittest.TestCase):
    def test_set_clipboard_text_raises_typed_open_error(self) -> None:
        user32 = FakeUser32(open_ok=False)
        kernel32 = FakeKernel32()

        with (
            patch.object(windows_input, "user32", user32),
            patch.object(windows_input, "kernel32", kernel32),
        ):
            with self.assertRaises(windows_input.ClipboardOpenError) as raised:
                windows_input.set_clipboard_text("hello")

        self.assertIsInstance(raised.exception, RuntimeError)
        self.assertEqual(str(raised.exception), "Failed to open the clipboard.")
        self.assertEqual(user32.close_count, 0)

    def test_set_clipboard_text_raises_typed_empty_and_alloc_errors(self) -> None:
        empty_user32 = FakeUser32(empty_ok=False)
        with (
            patch.object(windows_input, "user32", empty_user32),
            patch.object(windows_input, "kernel32", FakeKernel32()),
        ):
            with self.assertRaises(windows_input.ClipboardEmptyError) as empty:
                windows_input.set_clipboard_text("hello")

        self.assertEqual(str(empty.exception), "Failed to empty the clipboard.")
        self.assertEqual(empty_user32.close_count, 1)

        alloc_user32 = FakeUser32()
        with (
            patch.object(windows_input, "user32", alloc_user32),
            patch.object(windows_input, "kernel32", FakeKernel32(alloc_handle=0)),
        ):
            with self.assertRaises(windows_input.ClipboardAllocationError) as alloc:
                windows_input.set_clipboard_text("hello")

        self.assertEqual(str(alloc.exception), "GlobalAlloc failed.")
        self.assertEqual(alloc_user32.close_count, 1)

    def test_set_clipboard_text_raises_typed_lock_and_set_data_errors(self) -> None:
        locked_user32 = FakeUser32()
        locked_kernel32 = FakeKernel32(lock_pointer=0)
        with (
            patch.object(windows_input, "user32", locked_user32),
            patch.object(windows_input, "kernel32", locked_kernel32),
        ):
            with self.assertRaises(windows_input.ClipboardLockError) as locked:
                windows_input.set_clipboard_text("hello")

        self.assertEqual(str(locked.exception), "GlobalLock failed.")
        self.assertEqual(locked_kernel32.free_count, 1)
        self.assertEqual(locked_user32.close_count, 1)

        set_user32 = FakeUser32(set_handle=0)
        set_kernel32 = FakeKernel32()
        with (
            patch.object(windows_input, "user32", set_user32),
            patch.object(windows_input, "kernel32", set_kernel32),
            patch.object(ctypes, "memmove", return_value=0),
        ):
            with self.assertRaises(windows_input.ClipboardSetDataError) as set_data:
                windows_input.set_clipboard_text("hello")

        self.assertEqual(str(set_data.exception), "SetClipboardData failed.")
        self.assertEqual(set_kernel32.unlock_count, 1)
        self.assertEqual(set_kernel32.free_count, 1)
        self.assertEqual(set_user32.close_count, 1)


class FakeUser32:
    def __init__(self, *, open_ok: bool = True, empty_ok: bool = True, set_handle: int = 1) -> None:
        self.open_ok: bool = open_ok
        self.empty_ok: bool = empty_ok
        self.set_handle: int = set_handle
        self.close_count: int = 0

    def OpenClipboard(self, _hwnd: int | None) -> bool:
        return self.open_ok

    def EmptyClipboard(self) -> bool:
        return self.empty_ok

    def SetClipboardData(self, _format_id: int, _handle: int) -> int:
        return self.set_handle

    def CloseClipboard(self) -> bool:
        self.close_count += 1
        return True


class FakeKernel32:
    def __init__(self, *, alloc_handle: int = 20, lock_pointer: int = 30) -> None:
        self.alloc_handle: int = alloc_handle
        self.lock_pointer: int = lock_pointer
        self.unlock_count: int = 0
        self.free_count: int = 0

    def GlobalAlloc(self, _flags: int, _size: int) -> int:
        return self.alloc_handle

    def GlobalLock(self, _handle: int) -> int:
        return self.lock_pointer

    def GlobalUnlock(self, _handle: int) -> bool:
        self.unlock_count += 1
        return True

    def GlobalFree(self, _handle: int) -> int:
        self.free_count += 1
        return 0


if __name__ == "__main__":
    _ = unittest.main()
