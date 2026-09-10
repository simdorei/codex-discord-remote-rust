# pyright: reportAny=false, reportUnannotatedClassAttribute=false
from __future__ import annotations

import ctypes
import ctypes.wintypes as wt

USER32 = ctypes.WinDLL("user32", use_last_error=True)
KERNEL32 = ctypes.WinDLL("kernel32", use_last_error=True)
GDI32 = ctypes.WinDLL("gdi32", use_last_error=True)


class Rect(ctypes.Structure):
    _fields_ = [
        ("left", wt.LONG),
        ("top", wt.LONG),
        ("right", wt.LONG),
        ("bottom", wt.LONG),
    ]


class BitmapInfoHeader(ctypes.Structure):
    _fields_ = [
        ("size", wt.DWORD),
        ("width", wt.LONG),
        ("height", wt.LONG),
        ("planes", wt.WORD),
        ("bit_count", wt.WORD),
        ("compression", wt.DWORD),
        ("size_image", wt.DWORD),
        ("x_pixels_per_meter", wt.LONG),
        ("y_pixels_per_meter", wt.LONG),
        ("colors_used", wt.DWORD),
        ("colors_important", wt.DWORD),
    ]


class BitmapInfo(ctypes.Structure):
    _fields_ = [
        ("header", BitmapInfoHeader),
        ("colors", wt.DWORD * 3),
    ]


ULONG_PTR = wt.WPARAM


class MouseInput(ctypes.Structure):
    _fields_ = [
        ("dx", wt.LONG),
        ("dy", wt.LONG),
        ("mouse_data", wt.DWORD),
        ("flags", wt.DWORD),
        ("time", wt.DWORD),
        ("extra_info", ULONG_PTR),
    ]


class KeyboardInput(ctypes.Structure):
    _fields_ = [
        ("virtual_key", wt.WORD),
        ("scan_code", wt.WORD),
        ("flags", wt.DWORD),
        ("time", wt.DWORD),
        ("extra_info", ULONG_PTR),
    ]


class HardwareInput(ctypes.Structure):
    _fields_ = [
        ("message", wt.DWORD),
        ("parameter_low", wt.WORD),
        ("parameter_high", wt.WORD),
    ]


class InputValue(ctypes.Union):
    _fields_ = (
        ("mouse", MouseInput),
        ("keyboard", KeyboardInput),
        ("hardware", HardwareInput),
    )


class Input(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("value", InputValue)]


EnumWindowsCallback = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def configure_native_signatures() -> None:
    USER32.EnumWindows.argtypes = [EnumWindowsCallback, wt.LPARAM]
    USER32.EnumWindows.restype = wt.BOOL
    USER32.EnumChildWindows.argtypes = [wt.HWND, EnumWindowsCallback, wt.LPARAM]
    USER32.EnumChildWindows.restype = wt.BOOL
    USER32.IsWindow.argtypes = [wt.HWND]
    USER32.IsWindow.restype = wt.BOOL
    USER32.IsWindowVisible.argtypes = [wt.HWND]
    USER32.IsWindowVisible.restype = wt.BOOL
    USER32.GetWindowTextLengthW.argtypes = [wt.HWND]
    USER32.GetWindowTextLengthW.restype = ctypes.c_int
    USER32.GetWindowTextW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
    USER32.GetWindowTextW.restype = ctypes.c_int
    USER32.GetClassNameW.argtypes = [wt.HWND, wt.LPWSTR, ctypes.c_int]
    USER32.GetClassNameW.restype = ctypes.c_int
    USER32.GetWindowRect.argtypes = [wt.HWND, ctypes.POINTER(Rect)]
    USER32.GetWindowRect.restype = wt.BOOL
    USER32.GetForegroundWindow.restype = wt.HWND
    USER32.GetWindowThreadProcessId.argtypes = [wt.HWND, ctypes.POINTER(wt.DWORD)]
    USER32.GetWindowThreadProcessId.restype = wt.DWORD
    USER32.AttachThreadInput.argtypes = [wt.DWORD, wt.DWORD, wt.BOOL]
    USER32.AttachThreadInput.restype = wt.BOOL
    USER32.ShowWindow.argtypes = [wt.HWND, ctypes.c_int]
    USER32.SetForegroundWindow.argtypes = [wt.HWND]
    USER32.SetForegroundWindow.restype = wt.BOOL
    USER32.BringWindowToTop.argtypes = [wt.HWND]
    USER32.BringWindowToTop.restype = wt.BOOL
    USER32.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
    USER32.PostMessageW.restype = wt.BOOL
    USER32.SendMessageTimeoutW.argtypes = [
        wt.HWND,
        wt.UINT,
        wt.WPARAM,
        wt.LPARAM,
        wt.UINT,
        wt.UINT,
        ctypes.POINTER(ULONG_PTR),
    ]
    USER32.SendMessageTimeoutW.restype = wt.LPARAM
    USER32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
    USER32.SetCursorPos.restype = wt.BOOL
    USER32.mouse_event.argtypes = [wt.DWORD, wt.DWORD, wt.DWORD, wt.DWORD, ULONG_PTR]
    USER32.SendInput.argtypes = [wt.UINT, ctypes.POINTER(Input), ctypes.c_int]
    USER32.SendInput.restype = wt.UINT
    USER32.GetDC.argtypes = [wt.HWND]
    USER32.GetDC.restype = wt.HDC
    USER32.ReleaseDC.argtypes = [wt.HWND, wt.HDC]
    USER32.ReleaseDC.restype = ctypes.c_int
    USER32.PrintWindow.argtypes = [wt.HWND, wt.HDC, wt.UINT]
    USER32.PrintWindow.restype = wt.BOOL
    USER32.OpenClipboard.argtypes = [wt.HWND]
    USER32.GetClipboardData.restype = wt.HANDLE
    USER32.SetClipboardData.argtypes = [wt.UINT, wt.HANDLE]
    USER32.SetClipboardData.restype = wt.HANDLE
    KERNEL32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
    KERNEL32.OpenProcess.restype = wt.HANDLE
    KERNEL32.QueryFullProcessImageNameW.argtypes = [
        wt.HANDLE,
        wt.DWORD,
        wt.LPWSTR,
        ctypes.POINTER(wt.DWORD),
    ]
    KERNEL32.QueryFullProcessImageNameW.restype = wt.BOOL
    KERNEL32.CloseHandle.argtypes = [wt.HANDLE]
    KERNEL32.GetCurrentThreadId.restype = wt.DWORD
    KERNEL32.GlobalAlloc.argtypes = [wt.UINT, ctypes.c_size_t]
    KERNEL32.GlobalAlloc.restype = wt.HGLOBAL
    KERNEL32.GlobalLock.argtypes = [wt.HGLOBAL]
    KERNEL32.GlobalLock.restype = wt.LPVOID
    KERNEL32.GlobalUnlock.argtypes = [wt.HGLOBAL]
    KERNEL32.GlobalFree.argtypes = [wt.HGLOBAL]
    GDI32.CreateCompatibleDC.argtypes = [wt.HDC]
    GDI32.CreateCompatibleDC.restype = wt.HDC
    GDI32.CreateCompatibleBitmap.argtypes = [wt.HDC, ctypes.c_int, ctypes.c_int]
    GDI32.CreateCompatibleBitmap.restype = wt.HBITMAP
    GDI32.SelectObject.argtypes = [wt.HDC, wt.HGDIOBJ]
    GDI32.SelectObject.restype = wt.HGDIOBJ
    GDI32.BitBlt.argtypes = [
        wt.HDC,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        ctypes.c_int,
        wt.HDC,
        ctypes.c_int,
        ctypes.c_int,
        wt.DWORD,
    ]
    GDI32.BitBlt.restype = wt.BOOL
    GDI32.GetDIBits.argtypes = [
        wt.HDC,
        wt.HBITMAP,
        wt.UINT,
        wt.UINT,
        wt.LPVOID,
        ctypes.POINTER(BitmapInfo),
        wt.UINT,
    ]
    GDI32.GetDIBits.restype = ctypes.c_int
    GDI32.DeleteObject.argtypes = [wt.HGDIOBJ]
    GDI32.DeleteDC.argtypes = [wt.HDC]


configure_native_signatures()
