from __future__ import annotations

import hashlib
import io

from codex_remote_mcp_file_hash import HASH_CHUNK_BYTES, sha256_stream


class _BoundedReadStream(io.BytesIO):
    def __init__(self, content: bytes) -> None:
        super().__init__(content)
        self.read_sizes: list[int | None] = []

    def read(self, size: int | None = -1) -> bytes:
        self.read_sizes.append(size)
        if size is None or size < 0 or size > HASH_CHUNK_BYTES:
            raise AssertionError("hashing attempted an unbounded read")
        return super().read(size)


def test_sha256_stream_reads_large_content_in_bounded_chunks() -> None:
    content = b"x" * (HASH_CHUNK_BYTES * 3 + 17)
    stream = _BoundedReadStream(content)

    actual = sha256_stream(stream)

    assert actual == hashlib.sha256(content).hexdigest()
    assert len(stream.read_sizes) > 1
    assert all(
        size is not None and 0 <= size <= HASH_CHUNK_BYTES
        for size in stream.read_sizes
    )
