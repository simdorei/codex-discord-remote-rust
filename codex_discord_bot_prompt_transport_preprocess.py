from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from types import ModuleType
from typing import cast

import codex_discord_prompt_mapped_delivery as mapped_delivery
import codex_discord_prompt_rewrite as prompt_rewrite
import codex_pro_runtime_preflight as pro_preflight


def _no_stored_cwd(target_thread_id: str | None) -> str | None:
    _ = target_thread_id
    return None


def make_prompt_preprocessor(module: ModuleType) -> mapped_delivery.PromptPreprocessor:
    default_cwd = cast(Path, module.SCRIPT_DIR)
    log = cast(Callable[[str], None], module.log_line)
    get_thread_cwd = cast(
        Callable[[str | None], str | None],
        getattr(module, "get_thread_cwd", _no_stored_cwd),
    )
    runtime_preflight = cast(
        prompt_rewrite.RuntimePreflight,
        getattr(
            module,
            "PRO_RUNTIME_PREFLIGHT",
            pro_preflight.run_pro_runtime_preflight_with_recovery,
        ),
    )

    def preprocess(
        prompt: str,
        target_thread_id: str | None = None,
    ) -> mapped_delivery.PromptPreprocessResult:
        if not prompt_rewrite.is_pro_command(prompt):
            return mapped_delivery.keep_prompt(prompt)
        stored_cwd = get_thread_cwd(target_thread_id)
        cwd = Path(stored_cwd) if stored_cwd else default_cwd
        return prompt_rewrite.rewrite_prompt(
            prompt,
            target_thread_id,
            cwd=cwd,
            log=log,
            runtime_preflight=runtime_preflight,
        )

    return preprocess


def make_discord_origin_prompt_marker(
    module: ModuleType,
) -> mapped_delivery.DiscordOriginPromptMarker:
    return cast(
        mapped_delivery.DiscordOriginPromptMarker,
        module.mark_recent_discord_origin_prompt,
    )
