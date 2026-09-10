# pyright: reportAny=false
from __future__ import annotations

import ctypes

from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_capture import encode_bgra_png
from codex_remote_mcp_windows_native import GDI32, USER32, BitmapInfo, BitmapInfoHeader

BI_RGB = 0
DIB_RGB_COLORS = 0
PW_RENDERFULLCONTENT = 0x00000002


def capture_window(window_id: int, width: int, height: int) -> bytes:
    window_dc = USER32.GetDC(window_id)
    if not window_dc:
        raise ComputerControlError("Windows could not open the window for capture.")
    memory_dc = GDI32.CreateCompatibleDC(window_dc)
    bitmap = GDI32.CreateCompatibleBitmap(window_dc, width, height)
    if not memory_dc or not bitmap:
        _cleanup(window_id, window_dc, memory_dc, bitmap)
        raise ComputerControlError("Windows could not allocate screenshot resources.")
    previous = GDI32.SelectObject(memory_dc, bitmap)
    try:
        if not USER32.PrintWindow(window_id, memory_dc, PW_RENDERFULLCONTENT):
            raise ComputerControlError("Windows window capture failed.")
        pixels = _bitmap_pixels(memory_dc, bitmap, width, height)
        return encode_bgra_png(width, height, pixels)
    finally:
        if previous:
            GDI32.SelectObject(memory_dc, previous)
        _cleanup(window_id, window_dc, memory_dc, bitmap)


def _bitmap_pixels(memory_dc: int, bitmap: int, width: int, height: int) -> bytes:
    info = BitmapInfo(
        header=BitmapInfoHeader(
            size=ctypes.sizeof(BitmapInfoHeader),
            width=width,
            height=-height,
            planes=1,
            bit_count=32,
            compression=BI_RGB,
            size_image=width * height * 4,
        )
    )
    buffer = (ctypes.c_ubyte * (width * height * 4))()
    rows = GDI32.GetDIBits(
        memory_dc,
        bitmap,
        0,
        height,
        buffer,
        ctypes.byref(info),
        DIB_RGB_COLORS,
    )
    if rows != height:
        raise ComputerControlError("Windows returned incomplete screenshot data.")
    return bytes(buffer)


def _cleanup(window_id: int, window_dc: int, memory_dc: int, bitmap: int) -> None:
    if bitmap:
        GDI32.DeleteObject(bitmap)
    if memory_dc:
        GDI32.DeleteDC(memory_dc)
    if window_dc:
        USER32.ReleaseDC(window_id, window_dc)
