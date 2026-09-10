from __future__ import annotations

from pathlib import Path


SINGLE_BACKUP_LOG_LIMIT_BYTES = 500 * 1024


def rotate_single_backup_file(
    path: Path,
    *,
    max_bytes: int = SINGLE_BACKUP_LOG_LIMIT_BYTES,
    incoming_bytes: int = 0,
) -> None:
    try:
        if max_bytes <= 0:
            return
        current_path = Path(path)
        if not current_path.exists():
            return
        projected_size = current_path.stat().st_size + max(0, incoming_bytes)
        if projected_size <= max_bytes:
            return
        backup_path = current_path.with_name(current_path.name + ".bak")
        if backup_path.exists():
            backup_path.unlink()
        _ = current_path.replace(backup_path)
        current_path.touch()
    except OSError:
        return


def is_windows_file_lock_error(exc: BaseException) -> bool:
    text = str(exc)
    lowered = text.lower()
    return (
        "os error 32" in lowered
        or "being used by another process" in lowered
        or "used by another process" in lowered
        or "다른 프로세스가 파일을 사용 중" in text
    )
