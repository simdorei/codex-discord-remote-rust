from __future__ import annotations

import codex_discord_runtime as discord_runtime
import codex_discord_text_digest as discord_text_digest


def make_discord_origin_prompt_digest(
    target_thread_id: str | None,
    prompt: str,
) -> str:
    return discord_text_digest.make_text_digest(
        "discord-origin",
        discord_runtime.normalize_runner_key(target_thread_id),
        str(prompt or "").strip(),
    )


def cleanup_recent_discord_origin_prompts(
    recent_prompts: dict[str, float],
    *,
    ttl_seconds: float,
    now: float,
) -> None:
    expired = [
        digest
        for digest, seen_at in recent_prompts.items()
        if now - seen_at > ttl_seconds
    ]
    for digest in expired:
        _ = recent_prompts.pop(digest, None)


def mark_recent_discord_origin_prompt(
    recent_prompts: dict[str, float],
    target_thread_id: str | None,
    prompt: str,
    *,
    ttl_seconds: float,
    now: float,
) -> None:
    cleanup_recent_discord_origin_prompts(recent_prompts, ttl_seconds=ttl_seconds, now=now)
    recent_prompts[make_discord_origin_prompt_digest(target_thread_id, prompt)] = now


def should_skip_discord_origin_prompt(
    recent_prompts: dict[str, float],
    target_thread_id: str | None,
    text: str,
    *,
    ttl_seconds: float,
    now: float,
) -> bool:
    cleanup_recent_discord_origin_prompts(recent_prompts, ttl_seconds=ttl_seconds, now=now)
    digest = make_discord_origin_prompt_digest(target_thread_id, text)
    if digest not in recent_prompts:
        return False
    _ = recent_prompts.pop(digest, None)
    return True
