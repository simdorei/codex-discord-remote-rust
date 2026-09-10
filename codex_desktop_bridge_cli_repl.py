from __future__ import annotations

import shlex
from typing import Final

REPL_EXIT_COMMANDS: Final = {"exit", "quit"}
REPL_HELP_COMMAND: Final = "help"
REPL_KNOWN_COMMANDS: Final = {
    "list",
    "archived_list",
    "use",
    "status",
    "doctor",
    "focus",
    "new",
    "archive",
    "delete_archive",
    "tail",
    "open",
    "stop",
    "ask",
    "help",
    "exit",
    "quit",
}


def split_repl_command(line: str) -> list[str]:
    lexer = shlex.shlex(line, posix=False)
    lexer.whitespace_split = True
    lexer.commenters = ""
    tokens = list(lexer)
    cleaned: list[str] = []
    for token in tokens:
        if len(token) >= 2 and token[0] == token[-1] and token[0] in ("'", '"'):
            cleaned.append(token[1:-1])
        else:
            cleaned.append(token)
    return cleaned


def normalize_repl_argv(argv: list[str], *, original_line: str) -> list[str]:
    if not argv:
        return []
    normalized = list(argv)
    command = normalized[0].lower()
    if command not in REPL_KNOWN_COMMANDS and not normalized[0].startswith("-"):
        return ["ask", "--stream", "--include-commentary", original_line]
    if command != "ask":
        return normalized

    has_wait_mode = any(token in {"--background", "--foreground", "--no-wait"} for token in normalized[1:])
    has_stream_mode = any(token in {"--stream", "--no-stream"} for token in normalized[1:])
    has_commentary_mode = any(token in {"--include-commentary", "--no-commentary"} for token in normalized[1:])
    if not has_wait_mode:
        normalized.insert(1, "--foreground")
    if not has_stream_mode:
        normalized.insert(1, "--stream")
    if not has_commentary_mode:
        normalized.insert(1, "--include-commentary")
    return normalized
