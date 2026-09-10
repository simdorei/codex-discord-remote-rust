from __future__ import annotations

import base64
import hashlib
from pathlib import Path
from typing import Final

from codex_remote_mcp_checkpoint_transaction import (
    CheckpointTarget,
    checkpoint_transaction,
)
from codex_remote_mcp_checkpoints import (
    begin_checkpoint,
    finish_checkpoint,
)
from codex_remote_mcp_file_listing import (
    ProjectGlobLimitError,
    iter_bounded_project_glob,
)
from codex_remote_mcp_files import ProjectFileAccess, ProjectFileError
from simdorei_mcp_common.operation_outputs import (
    ImageEntry,
    ImageListOutput,
    ImageRetrieveOutput,
    ImageSaveOutput,
)
from simdorei_mcp_common.request_deadlines import (
    RequestBudget,
    RequestDeadlineExpired,
)

MAX_IMAGE_BYTES: Final = 5_242_880
MAX_IMAGE_RESULTS: Final = 200
IMAGE_SKIP_DIRECTORIES: Final = frozenset(
    {
        ".codex-remote-mcp",
        ".git",
        ".next",
        ".venv",
        "__pycache__",
        "build",
        "dist",
        "node_modules",
        "vendor",
    }
)
IMAGE_TYPES: Final = {
    ".gif": ("image/gif", (b"GIF87a", b"GIF89a")),
    ".jpg": ("image/jpeg", (b"\xff\xd8\xff",)),
    ".jpeg": ("image/jpeg", (b"\xff\xd8\xff",)),
    ".png": ("image/png", (b"\x89PNG\r\n\x1a\n",)),
    ".webp": ("image/webp", (b"RIFF",)),
}


class ProjectImageError(ProjectFileError):
    """Raised when project image input is invalid or unsafe."""


def save_image(
    root: Path,
    path: str,
    data_base64: str,
    *,
    overwrite: bool,
    budget: RequestBudget,
) -> ImageSaveOutput:
    """Decode, validate, and atomically store one project image."""
    try:
        content = base64.b64decode(data_base64, validate=True)
    except ValueError as exc:
        raise ProjectImageError(path, "image data is not valid base64") from exc
    return _save_bytes(
        root,
        path,
        content,
        overwrite=overwrite,
        budget=budget,
    )


def save_image_from_url(
    root: Path,
    path: str,
    url: str,
    *,
    overwrite: bool,
    budget: RequestBudget,
) -> ImageSaveOutput:
    """Fetch one public HTTPS image and store it inside the project."""
    try:
        import httpx2

        from codex_remote_mcp_http import (
            PublicNetworkError,
            public_http_client,
            validate_public_url_shape,
        )
    except ModuleNotFoundError as exc:
        if exc.name not in {"httpcore2", "httpx2"}:
            raise
        raise ProjectImageError(
            "<image-url>",
            "public URL image support requires optional dependencies; run install.ps1",
        ) from exc

    content = bytearray()
    try:
        budget.ensure_active()
        validate_public_url_shape(url)
        with (
            public_http_client() as client,
            client.stream("GET", url) as response,
        ):
            _ = response.raise_for_status()
            for chunk in response.iter_bytes():
                content.extend(chunk)
                budget.ensure_active()
                if len(content) > MAX_IMAGE_BYTES:
                    raise ProjectImageError(
                        path,
                        f"image exceeds {MAX_IMAGE_BYTES} bytes",
                    )
    except RequestDeadlineExpired:
        raise
    except ProjectImageError:
        raise
    except PublicNetworkError as exc:
        raise ProjectImageError("<image-url>", str(exc)) from exc
    except (OSError, httpx2.HTTPError) as exc:
        raise ProjectImageError(
            "<image-url>",
            f"image download failed: {type(exc).__name__}",
        ) from exc
    budget.ensure_active()
    return _save_bytes(
        root,
        path,
        bytes(content),
        overwrite=overwrite,
        budget=budget,
    )


def list_images(root: Path, *, budget: RequestBudget) -> ImageListOutput:
    """List bounded, validated image files visible inside the project."""
    access = ProjectFileAccess(root)
    entries: list[ImageEntry] = []
    try:
        candidates = iter_bounded_project_glob(
            access.root,
            "**/*",
            excluded_directory_names=IMAGE_SKIP_DIRECTORIES,
            ensure_active=budget.ensure_active,
        )
        for candidate in candidates:
            budget.ensure_active()
            if len(entries) >= MAX_IMAGE_RESULTS:
                break
            if candidate.suffix.casefold() not in IMAGE_TYPES:
                continue
            relative = candidate.relative_to(access.root).as_posix()
            try:
                if access.file_size(relative) > MAX_IMAGE_BYTES:
                    continue
                content = access.read_bytes(relative, max_bytes=MAX_IMAGE_BYTES)
                entries.append(_image_entry(relative, content))
            except ProjectFileError:
                continue
    except ProjectGlobLimitError as exc:
        raise ProjectImageError("<images>", str(exc)) from exc
    return ImageListOutput(images=tuple(entries))


def retrieve_image(
    root: Path,
    path: str,
    *,
    budget: RequestBudget,
) -> ImageRetrieveOutput:
    """Return one bounded project image as base64."""
    access = ProjectFileAccess(root)
    target = access.resolve_path(path)
    budget.ensure_active()
    if access.file_size(path) > MAX_IMAGE_BYTES:
        raise ProjectImageError(path, f"image exceeds {MAX_IMAGE_BYTES} bytes")
    content = access.read_bytes(path, max_bytes=MAX_IMAGE_BYTES)
    budget.ensure_active()
    _ = _validate_image(path, content)
    image = _image_entry(target.relative_to(access.root).as_posix(), content)
    return ImageRetrieveOutput(
        image=image,
        data_base64=base64.b64encode(content).decode("ascii"),
    )


def _save_bytes(
    root: Path,
    path: str,
    content: bytes,
    *,
    overwrite: bool,
    budget: RequestBudget,
) -> ImageSaveOutput:
    access = ProjectFileAccess(root)
    target = access.resolve_path(path, require_file=False)
    exists = access.file_exists(path)
    if exists and not overwrite:
        raise ProjectImageError(path, "file already exists")
    if exists and access.file_size(path) > MAX_IMAGE_BYTES:
        raise ProjectImageError(path, f"image exceeds {MAX_IMAGE_BYTES} bytes")
    _ = _validate_image(path, content)
    budget.ensure_active()
    draft = begin_checkpoint(
        root,
        "save image",
        (CheckpointTarget(path=path, absolute_path=target),),
        max_target_bytes=MAX_IMAGE_BYTES,
    )
    with checkpoint_transaction(draft) as transaction:
        expected = (
            hashlib.sha256(
                access.read_bytes(path, max_bytes=MAX_IMAGE_BYTES)
            ).hexdigest()
            if exists
            else None
        )
        budget.ensure_active()
        _ = access.write_bytes(path, content, expected_sha256=expected)
        transaction.record_write(path, hashlib.sha256(content).hexdigest())
        _ = finish_checkpoint(draft)
    relative = target.relative_to(access.root).as_posix()
    return ImageSaveOutput(
        image=_image_entry(relative, content),
        sha256=hashlib.sha256(content).hexdigest(),
    )


def _validate_image(path: str, content: bytes) -> str:
    if len(content) > MAX_IMAGE_BYTES:
        raise ProjectImageError(path, f"image exceeds {MAX_IMAGE_BYTES} bytes")
    suffix = Path(path).suffix.casefold()
    image_type = IMAGE_TYPES.get(suffix)
    if image_type is None:
        raise ProjectImageError(path, "supported types are PNG, JPEG, GIF, and WebP")
    media_type, signatures = image_type
    if not any(content.startswith(signature) for signature in signatures):
        raise ProjectImageError(path, "file content does not match its image type")
    if suffix == ".webp" and content[8:12] != b"WEBP":
        raise ProjectImageError(path, "file content does not match WebP")
    return media_type


def _image_entry(relative: str, content: bytes) -> ImageEntry:
    size = len(content)
    if size > MAX_IMAGE_BYTES:
        raise ProjectImageError(relative, f"image exceeds {MAX_IMAGE_BYTES} bytes")
    return ImageEntry(
        path=relative,
        media_type=_validate_image(relative, content),
        size_bytes=size,
    )
