from __future__ import annotations

import struct
import zlib

from codex_remote_mcp_computer_errors import ComputerControlError

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def encode_bgra_png(width: int, height: int, bgra: bytes) -> bytes:
    if width <= 0 or height <= 0:
        raise ComputerControlError("Screenshot dimensions must be positive.")
    expected = width * height * 4
    if len(bgra) != expected:
        raise ComputerControlError("Windows returned incomplete screenshot pixels.")
    scanlines = bytearray()
    stride = width * 4
    for row_index in range(height):
        row = bgra[row_index * stride : (row_index + 1) * stride]
        rgb = bytearray(width * 3)
        rgb[0::3] = row[2::4]
        rgb[1::3] = row[1::4]
        rgb[2::3] = row[0::4]
        scanlines.append(0)
        scanlines.extend(rgb)
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"".join(
        (
            PNG_SIGNATURE,
            _chunk(b"IHDR", header),
            _chunk(b"IDAT", zlib.compress(bytes(scanlines), level=6)),
            _chunk(b"IEND", b""),
        )
    )


def _chunk(kind: bytes, payload: bytes) -> bytes:
    checksum = zlib.crc32(kind)
    checksum = zlib.crc32(payload, checksum) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)
