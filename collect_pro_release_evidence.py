from __future__ import annotations

import argparse
import sys
from collections.abc import Sequence
from pathlib import Path
from typing import Protocol, cast

from codex_pro_release_checks import collect_current_release_evidence
from codex_pro_release_evidence import write_evidence_artifacts


DEFAULT_OUTPUT = Path(".release-evidence/pro-release-evidence.json")


class _ParsedArgs(Protocol):
    repo_root: Path
    output: Path


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Collect fail-closed pre-restart evidence for the !pro release path."
    )
    _ = parser.add_argument("--repo-root", type=Path, default=Path(__file__).parent)
    _ = parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = cast(_ParsedArgs, cast(object, _parser().parse_args(argv)))
    repo_root = args.repo_root.resolve()
    output = args.output
    if not output.is_absolute():
        output = repo_root / output
    evidence = collect_current_release_evidence(repo_root)
    json_path, summary_path = write_evidence_artifacts(evidence, output)
    print(evidence.summary(), end="")
    print(f"JSON: {json_path}")
    print(f"Summary: {summary_path}")
    return 0 if evidence.pre_restart_ready else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
