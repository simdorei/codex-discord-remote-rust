from __future__ import annotations

import argparse
from collections.abc import Callable
from dataclasses import dataclass
from typing import cast

import codex_desktop_bridge_cli as bridge_cli


BuildParser = Callable[[], argparse.ArgumentParser]
CommandFunc = Callable[[argparse.Namespace], int | None]
GetSelectedThreadId = Callable[[], str | None]
InputLine = Callable[[str], str]
PrintLine = Callable[[str], None]
SplitReplCommand = Callable[[str], list[str]]


@dataclass(frozen=True, slots=True)
class ReplDeps:
    get_selected_thread_id: GetSelectedThreadId
    build_parser: BuildParser
    split_repl_command: SplitReplCommand
    input_line: InputLine
    print_line: PrintLine


def run_repl(deps: ReplDeps) -> int:
    _print_repl_intro(deps.print_line)
    while True:
        try:
            line = _read_repl_line(deps).strip()
        except EOFError:
            deps.print_line("")
            return 0
        except KeyboardInterrupt:
            deps.print_line("")
            return 130

        if not line:
            continue
        result = _handle_repl_line(line, deps)
        if result is not None:
            return result


def _print_repl_intro(print_line: PrintLine) -> None:
    print_line("Codex bridge shell")
    print_line("Commands: list, archived_list, open, use, new, archive, delete_archive, ask, status, doctor, tail, focus, help, exit")
    print_line('Primary flow: list -> open ai -> ask "..."')
    print_line("Example: open ai")
    print_line('Example: new "테스트"')
    print_line("Example: archive other")
    print_line("Example: archived_list")
    print_line("Example: delete_archive 1")
    print_line("Example: open --abort ai")
    print_line("Example: open other")
    print_line("Example: doctor")
    print_line('Example: ask "이 파일 수정해줘"')
    print_line("`open` selects + opens a thread. `use` only selects without opening.")
    print_line('Tip: plain text is treated as `ask --stream --include-commentary "..."`')
    print_line("Busy safety: `open` is blocked while another reply is running unless you pass `--abort`.")
    print_line("Default ask uses background IPC. Pass `--ui` to use the legacy foreground paste path.")
    print_line("")


def _read_repl_line(deps: ReplDeps) -> str:
    selected = deps.get_selected_thread_id()
    suffix = f"[{selected[:8]}]" if selected else ""
    return deps.input_line(f"codex-bridge{suffix}> ")


def _handle_repl_line(line: str, deps: ReplDeps) -> int | None:
    lowered = line.lower()
    if lowered in bridge_cli.REPL_EXIT_COMMANDS:
        return 0

    if lowered == bridge_cli.REPL_HELP_COMMAND:
        deps.build_parser().print_help()
        deps.print_line("")
        return None

    argv = bridge_cli.normalize_repl_argv(deps.split_repl_command(line), original_line=line)
    if not argv:
        return None

    parser = deps.build_parser()
    try:
        args = parser.parse_args(argv)
        func = cast(CommandFunc, getattr(args, "func"))
        exit_code = func(args)
        if exit_code not in (0, None):
            deps.print_line(f"(exit {exit_code})")
    except SystemExit as exc:
        code = exc.code if isinstance(exc.code, int) else 1
        if code != 0:
            deps.print_line("(invalid command)")
    except KeyboardInterrupt:
        deps.print_line("Interrupted.")
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        deps.print_line(f"ERROR: {exc}")
    deps.print_line("")
    return None
