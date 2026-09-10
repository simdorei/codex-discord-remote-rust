from __future__ import annotations

import struct

from codex_remote_mcp_windows_capture import encode_bgra_png


def test_encode_bgra_png_writes_valid_png_dimensions() -> None:
    png = encode_bgra_png(2, 1, bytes((0, 0, 255, 0, 0, 255, 0, 0)))

    assert png.startswith(b"\x89PNG\r\n\x1a\n")
    assert struct.unpack(">II", png[16:24]) == (2, 1)
    assert png.endswith(b"IEND\xaeB`\x82")
