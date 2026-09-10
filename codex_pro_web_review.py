from __future__ import annotations
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from datetime import datetime, timezone
import os
from pathlib import Path
import re
from typing import Final, Protocol
DEFAULT_OUT_DIR: Final = ".pro-web-review"
DEFAULT_MODEL_LABEL: Final = "GPT-5.5 Pro"
DEFAULT_CDP_URL: Final = "http://127.0.0.1:9222"
_SKIP_DIRS: Final = {
    ".git",
    ".hg",
    ".insane-review",
    ".mypy_cache",
    ".pro-web-review",
    ".pytest_cache",
    ".ruff_cache",
    ".venv",
    "__pycache__",
    "node_modules",
    "venv",
}
_SKIP_NAMES: Final = {".env", ".env.local", ".env.production"}
_TEXT_SUFFIXES: Final = frozenset(
    {
        ".bat", ".cmd", ".css", ".go", ".html", ".js", ".json", ".md", ".mts", ".ps1", ".py",
        ".pyi", ".rs", ".sh", ".sql", ".toml", ".ts", ".tsx", ".txt", ".xml", ".yaml", ".yml",
    }
)
class ProWebReviewError(RuntimeError):
    pass
class ProWebReviewTargetError(ProWebReviewError):
    pass
class ProWebReviewPackError(ProWebReviewError):
    pass
class ProWebReviewBrowserError(ProWebReviewError):
    pass
class ProWebReviewLoginRequiredError(ProWebReviewBrowserError):
    pass
class ProWebReviewEmptyResponseError(ProWebReviewError):
    pass
@dataclass(frozen=True, slots=True)
class ProWebReviewRequest:
    root: Path
    target: str
    prompt: str
    include: tuple[str, ...] = ()
    out_dir: Path = Path(DEFAULT_OUT_DIR)
    model_label: str = DEFAULT_MODEL_LABEL
@dataclass(frozen=True, slots=True)
class ProWebReviewPack:
    target: str
    pack_path: Path
    prompt_text: str
    packed_files: tuple[Path, ...]
@dataclass(frozen=True, slots=True)
class ProWebReviewResult:
    pack: ProWebReviewPack
    response_path: Path
    model_label: str
class ProWebReviewer(Protocol):
    def submit_review(self, pack: ProWebReviewPack) -> str:
        ...
def pack_pro_web_review(
    request: ProWebReviewRequest,
    *,
    now: Callable[[], datetime] | None = None,
) -> ProWebReviewPack:
    root = request.root.expanduser().resolve()
    target_path = _resolve_under_root(root, request.target)
    out_dir = _resolve_out_dir(root, request.out_dir)
    stamp = _timestamp(now)
    slug = _slug(target_path.name or root.name)
    files = _collect_files(root, target_path, request.include)
    if not files:
        raise ProWebReviewPackError(f"No reviewable text files found for target: {target_path}")
    pack_path = out_dir / f"pack_{slug}_{stamp}.md"
    pack_text = _render_pack(root, target_path, files, now)
    _atomic_write_private(pack_path, pack_text)
    return ProWebReviewPack(
        target=str(target_path.relative_to(root)),
        pack_path=pack_path,
        prompt_text=_review_prompt(request.prompt, target_path.relative_to(root), request.model_label),
        packed_files=tuple(files),
    )
def run_pro_web_review(
    request: ProWebReviewRequest,
    reviewer: ProWebReviewer,
    *,
    now: Callable[[], datetime] | None = None,
) -> ProWebReviewResult:
    pack = pack_pro_web_review(request, now=now)
    response = reviewer.submit_review(pack).strip()
    if not response:
        raise ProWebReviewEmptyResponseError("ChatGPT returned an empty review response.")
    response_path = save_pro_web_review_response(
        pack=pack,
        response_text=response,
        out_dir=_resolve_out_dir(request.root.expanduser().resolve(), request.out_dir),
        model_label=request.model_label,
        now=now,
    )
    return ProWebReviewResult(pack=pack, response_path=response_path, model_label=request.model_label)
def save_pro_web_review_response(
    *,
    pack: ProWebReviewPack,
    response_text: str,
    out_dir: Path,
    model_label: str,
    now: Callable[[], datetime] | None = None,
) -> Path:
    stamp = _timestamp(now)
    response_path = out_dir / f"response_{_slug(pack.target)}_{stamp}.md"
    body = "\n".join(
        [
            f"# Pro Web Review: {pack.target}",
            "",
            f"- model: {model_label}",
            f"- pack: {pack.pack_path.name}",
            "",
            "---",
            "",
            response_text.strip(),
            "",
        ]
    )
    _atomic_write_private(response_path, body)
    return response_path
def cdp_login_help(cdp_url: str = DEFAULT_CDP_URL) -> str:
    return _login_help(cdp_url)
def _collect_files(root: Path, target_path: Path, include: Sequence[str]) -> list[Path]:
    if include:
        files = [_resolve_include(root, pattern) for pattern in include]
        flattened = [path for group in files for path in group]
    elif target_path.is_file():
        flattened = [target_path]
    elif target_path.is_dir():
        flattened = [path for path in target_path.rglob("*") if path.is_file()]
    else:
        raise ProWebReviewTargetError(f"Review target is not a file or directory: {target_path}")
    return sorted({path for path in flattened if _is_reviewable_file(root, path)})
def _resolve_include(root: Path, pattern: str) -> list[Path]:
    if Path(pattern).is_absolute():
        raise ProWebReviewTargetError(f"Include patterns must be relative to the root: {pattern}")
    matches = [path.resolve() for path in root.glob(pattern) if path.is_file()]
    for path in matches:
        _ensure_under_root(root, path)
    return matches
def _resolve_under_root(root: Path, value: str) -> Path:
    path = Path(value).expanduser()
    resolved = path.resolve() if path.is_absolute() else (root / path).resolve()
    _ensure_under_root(root, resolved)
    if not resolved.exists():
        raise ProWebReviewTargetError(f"Review target does not exist: {resolved}")
    return resolved
def _ensure_under_root(root: Path, path: Path) -> None:
    try:
        path.relative_to(root)
    except ValueError as exc:
        raise ProWebReviewTargetError(f"Path is outside review root: {path}") from exc
def _resolve_out_dir(root: Path, out_dir: Path) -> Path:
    return out_dir.expanduser() if out_dir.is_absolute() else root / out_dir
def _is_reviewable_file(root: Path, path: Path) -> bool:
    try:
        rel = path.relative_to(root)
    except ValueError:
        return False
    if path.name in _SKIP_NAMES or any(part in _SKIP_DIRS for part in rel.parts):
        return False
    return path.suffix.lower() in _TEXT_SUFFIXES
def _render_pack(
    root: Path,
    target_path: Path,
    files: Sequence[Path],
    now: Callable[[], datetime] | None,
) -> str:
    generated_at = _current_time(now).isoformat()
    lines = [
        "# Pro Web Review Pack",
        "",
        f"- target: {target_path.relative_to(root)}",
        f"- generated_at: {generated_at}",
        f"- files: {len(files)}",
        "",
    ]
    for path in files:
        lines.extend(_render_file(root, path))
    return "\n".join(lines).rstrip() + "\n"
def _render_file(root: Path, path: Path) -> list[str]:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        raise ProWebReviewPackError(f"Failed to read review file {path}: {exc}") from exc
    rel = path.relative_to(root)
    lines = ["", f"## File: {rel}", "", "```text"]
    for index, line in enumerate(text.splitlines(), start=1):
        lines.append(f"{index}\t{line}")
    lines.append("```")
    return lines
def _review_prompt(prompt: str, target: Path, model_label: str) -> str:
    return "\n".join(
        [
            f"Use the attached code pack to review `{target}` with {model_label}.",
            "Find concrete bugs, regressions, security issues, and missing tests.",
            "Cite file paths and line numbers from the pack. Do not invent files.",
            "Put findings first, ordered by severity, then short open questions.",
            "",
            "User request:",
            prompt.strip(),
        ]
    ).strip()
def _atomic_write_private(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = path.with_name(f".{path.name}.tmp")
    try:
        tmp_path.write_text(text, encoding="utf-8")
        os.chmod(tmp_path, 0o600)
        os.replace(tmp_path, path)
    except OSError as exc:
        raise ProWebReviewPackError(f"Failed to write review artifact {path}: {exc}") from exc
def _login_help(cdp_url: str) -> str:
    return (
        "Codex login and ChatGPT browser login are separate. "
        f"Open Chrome with remote debugging for {cdp_url}, sign in at chatgpt.com, "
        "select the Pro model, then retry."
    )
def _timestamp(now: Callable[[], datetime] | None) -> str:
    return _current_time(now).strftime("%Y%m%d_%H%M%S")
def _current_time(now: Callable[[], datetime] | None) -> datetime:
    value = now() if now is not None else datetime.now(timezone.utc)
    return value if value.tzinfo is not None else value.replace(tzinfo=timezone.utc)
def _slug(value: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9_.-]+", "-", value).strip("-._")
    return slug or "review"
