from __future__ import annotations

import ctypes
from typing import Final

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_native import USER32

WM_GETTEXT: Final = 0x000D
WM_GETTEXTLENGTH: Final = 0x000E
SMTO_ABORTIFHUNG: Final = 0x0002
MESSAGE_TIMEOUT_MS: Final = 1_000


def read_control_text(window_id: int, *, limit: int) -> str:
    """Read a cross-process classic control through a bounded system message."""
    length = _send_message_timeout(window_id, WM_GETTEXTLENGTH)
    if length <= 0:
        return ""
    if length > limit:
        raise ComputerControlError("The Notepad document is too large to control safely.")
    buffer = ctypes.create_unicode_buffer(length + 1)
    pointer = ctypes.cast(buffer, ctypes.c_void_p).value
    if pointer is None:
        raise ComputerControlError("Windows could not prepare the Notepad content check.")
    _ = _send_message_timeout(
        window_id,
        WM_GETTEXT,
        length + 1,
        pointer,
    )
    return buffer.value


def _send_message_timeout(
    window_id: int,
    message: int,
    wparam: int = 0,
    lparam: int = 0,
) -> int:
    result = ctypes.c_size_t()
    sent = USER32.SendMessageTimeoutW(
        window_id,
        message,
        wparam,
        lparam,
        SMTO_ABORTIFHUNG,
        MESSAGE_TIMEOUT_MS,
        ctypes.byref(result),
    )
    if not sent:
        raise ComputerControlError("The Notepad editor did not answer safely in time.")
    return int(result.value)
