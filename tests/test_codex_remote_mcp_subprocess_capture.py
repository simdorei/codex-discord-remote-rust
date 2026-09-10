from __future__ import annotations

from codex_remote_mcp_subprocess_capture import (
    TRUNCATION_MARKER,
    BoundedProcessCapture,
)


def test_capture_keeps_output_at_exact_limit_complete() -> None:
    capture = BoundedProcessCapture(limit=64)

    capture.append(b"a" * 32)
    capture.append(b"b" * 32)

    assert capture.value() == (b"a" * 32) + (b"b" * 32)
    assert capture.total_bytes == 64
    assert not capture.truncated


def test_capture_truncates_one_byte_over_limit_to_exact_size() -> None:
    capture = BoundedProcessCapture(limit=64)

    capture.append(b"a" * 64)
    capture.append(b"b")

    value = capture.value()
    assert len(value) == 64
    assert TRUNCATION_MARKER in value
    assert capture.total_bytes == 65
    assert capture.truncated


def test_capture_preserves_split_multibyte_utf8_when_under_limit() -> None:
    encoded = "가나다라마바사".encode("utf-8")
    capture = BoundedProcessCapture(limit=len(encoded))

    for byte in encoded:
        capture.append(bytes((byte,)))

    assert capture.value().decode("utf-8") == "가나다라마바사"
    assert not capture.truncated
