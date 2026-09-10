from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import unittest
from unittest.mock import patch

import codex_desktop_bridge_ipc_pipe as ipc_pipe


class IpcPipeErrorTests(unittest.TestCase):
    def test_open_pipe_raises_typed_error(self) -> None:
        with (
            patch.object(ipc_pipe.kernel32, "CreateFileW", return_value=ipc_pipe.INVALID_HANDLE_VALUE),
            patch.object(ipc_pipe, "get_last_win_error_message", return_value="denied"),
        ):
            with self.assertRaises(ipc_pipe.IPCPipeOpenError) as raised:
                _ = ipc_pipe.open_codex_ipc_pipe()

        self.assertIsInstance(raised.exception, RuntimeError)
        self.assertEqual(str(raised.exception), r"Could not open \\.\pipe\codex-ipc: denied")

    def test_peek_pipe_raises_typed_error(self) -> None:
        with (
            patch.object(ipc_pipe.kernel32, "PeekNamedPipe", return_value=0),
            patch.object(ipc_pipe, "get_last_win_error_message", return_value="no pipe"),
        ):
            with self.assertRaises(ipc_pipe.IPCPipePeekError) as raised:
                _ = ipc_pipe.peek_pipe_bytes_available(5)

        self.assertEqual(str(raised.exception), r"Could not peek \\.\pipe\codex-ipc: no pipe")

    def test_read_pipe_exact_raises_typed_read_errors(self) -> None:
        with (
            patch.object(ipc_pipe, "wait_for_pipe_bytes", return_value=None),
            patch.object(ipc_pipe.kernel32, "ReadFile", return_value=0),
            patch.object(ipc_pipe, "get_last_win_error_message", return_value="closed"),
        ):
            with self.assertRaises(ipc_pipe.IPCPipeReadError) as failed:
                _ = ipc_pipe.read_pipe_exact(5, 4, 1.0)

        self.assertEqual(str(failed.exception), r"Could not read from \\.\pipe\codex-ipc: closed")

        with (
            patch.object(ipc_pipe, "wait_for_pipe_bytes", return_value=None),
            patch.object(ctypes, "byref", side_effect=_byref_with_two),
            patch.object(ipc_pipe.kernel32, "ReadFile", return_value=1),
        ):
            with self.assertRaises(ipc_pipe.IPCShortReadError) as short:
                _ = ipc_pipe.read_pipe_exact(5, 4, 1.0)

        self.assertEqual(str(short.exception), r"Short IPC read from \\.\pipe\codex-ipc: expected 4, got 2.")
        self.assertEqual(short.exception.expected_size, 4)
        self.assertEqual(short.exception.actual_size, 2)

    def test_read_ipc_message_rejects_non_object_payload(self) -> None:
        with patch.object(ipc_pipe, "read_pipe_exact", side_effect=[(2).to_bytes(4, "little"), b"[]"]):
            with self.assertRaises(ipc_pipe.IPCInvalidMessageError) as raised:
                _ = ipc_pipe.read_ipc_message(5, 1.0)

        self.assertEqual(str(raised.exception), "IPC message was not a JSON object.")

    def test_write_ipc_message_raises_typed_write_errors(self) -> None:
        with (
            patch.object(ipc_pipe.kernel32, "WriteFile", return_value=0),
            patch.object(ipc_pipe, "get_last_win_error_message", return_value="broken"),
        ):
            with self.assertRaises(ipc_pipe.IPCPipeWriteError) as failed:
                ipc_pipe.write_ipc_message(5, {"ok": True})

        self.assertEqual(str(failed.exception), r"Could not write to \\.\pipe\codex-ipc: broken")

        with (
            patch.object(ctypes, "byref", side_effect=_byref_with_two),
            patch.object(ipc_pipe.kernel32, "WriteFile", return_value=1),
        ):
            with self.assertRaises(ipc_pipe.IPCShortWriteError) as short:
                ipc_pipe.write_ipc_message(5, {"ok": True})

        self.assertEqual(short.exception.actual_size, 2)
        self.assertIn("Short IPC write", str(short.exception))


def _byref_with_two(value: wt.DWORD) -> wt.DWORD:
    value.value = 2
    return value


if __name__ == "__main__":
    _ = unittest.main()
