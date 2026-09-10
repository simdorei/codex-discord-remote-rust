from __future__ import annotations

from pathlib import Path
from typing import Final

from codex_remote_mcp_file_listing import (
    ProjectGlobLimitError,
    iter_bounded_project_glob,
)
from codex_remote_mcp_files import (
    MAX_FILE_BYTES,
    ProjectFileAccess,
    ProjectFileError,
    ProjectFileSizeError,
    UnsafeProjectPathError,
)
from codex_remote_mcp_redaction import redact
from simdorei_mcp_common.operation_outputs import (
    CodeSearchOutput,
    ProjectRulesOutput,
    RuleFile,
    SearchMatch,
)
from simdorei_mcp_common.operation_requests import CodeSearchRequest
from simdorei_mcp_common.request_deadlines import RequestBudget

RULE_FILES: Final = ("AGENTS.md", "CLAUDE.md", ".codex/config.toml")
SKIP_DIRECTORIES: Final = frozenset(
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
MAX_RULE_CHARACTERS: Final = 20_000


def read_project_rules(
    root: Path,
    *,
    budget: RequestBudget,
) -> ProjectRulesOutput:
    """Read bounded project rule files through the normal path guard."""
    access = ProjectFileAccess(root)
    rules: list[RuleFile] = []
    for candidate in RULE_FILES:
        budget.ensure_active()
        try:
            raw = access.read_bytes(
                candidate,
                max_bytes=MAX_FILE_BYTES,
                allow_truncated=True,
            )
        except (ProjectFileSizeError, UnsafeProjectPathError):
            continue
        try:
            content = raw.decode("utf-8")
        except UnicodeDecodeError:
            continue
        rules.append(
            RuleFile(
                path=candidate,
                content=redact(content[:MAX_RULE_CHARACTERS]),
            )
        )
    return ProjectRulesOutput(rules=tuple(rules))


def search_project(
    root: Path,
    request: CodeSearchRequest,
    *,
    budget: RequestBudget,
) -> CodeSearchOutput:
    """Search UTF-8 project files without following links or secret paths."""
    access = ProjectFileAccess(root)
    matches: list[SearchMatch] = []
    try:
        candidates = iter_bounded_project_glob(
            access.root,
            "**/*",
            excluded_directory_names=SKIP_DIRECTORIES,
            ensure_active=budget.ensure_active,
        )
        for candidate in candidates:
            budget.ensure_active()
            if len(matches) >= request.max_results:
                break
            relative = candidate.relative_to(access.root).as_posix()
            try:
                if access.file_size(relative) > MAX_FILE_BYTES:
                    continue
                raw = access.read_bytes(relative, max_bytes=MAX_FILE_BYTES)
            except ProjectFileError:
                continue
            if b"\0" in raw:
                continue
            try:
                content = raw.decode("utf-8")
            except UnicodeDecodeError:
                continue
            for line_number, line in enumerate(content.splitlines(), start=1):
                if request.query not in line:
                    continue
                matches.append(
                    SearchMatch(
                        path=relative,
                        line=line_number,
                        snippet=redact(line.strip()[:400]),
                    )
                )
                if len(matches) >= request.max_results:
                    break
    except ProjectGlobLimitError as exc:
        raise UnsafeProjectPathError("<search>", str(exc)) from exc
    return CodeSearchOutput(matches=tuple(matches))
