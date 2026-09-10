from __future__ import annotations

import hashlib
from pathlib import Path
from typing import BinaryIO, Final

HASH_CHUNK_BYTES: Final = 64 * 1024


def sha256_stream(stream: BinaryIO) -> str:
    digest = hashlib.sha256()
    while chunk := stream.read(HASH_CHUNK_BYTES):
        digest.update(chunk)
    return digest.hexdigest()


def sha256_path(path: Path) -> str:
    with path.open("rb") as stream:
        return sha256_stream(stream)
