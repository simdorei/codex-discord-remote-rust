from __future__ import annotations

import asyncio  # noqa: ANYIO_OK -- Discord.py exposes asyncio-based boundaries.
from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass
from typing import Generic, Protocol, TypeVar

from codex_pro_runtime_diagnostics import ProRuntimeDiagnostic

ChannelContraT = TypeVar("ChannelContraT", contravariant=True)
ChannelT = TypeVar("ChannelT")
LogFunc = Callable[[str], None]
TextLenFunc = Callable[[str | None], int]
OutputPredicate = Callable[[str], bool]
BusyPredicate = Callable[[int, str], bool]
PendingFormatter = Callable[[str], str]
OutputTargetReleaser = Callable[[str | None], Awaitable[bool]]
SelectedThreadSetter = Callable[[str], None]
DiscordOriginPromptMarker = Callable[[str | None, str], None]


@dataclass(frozen=True, slots=True)
class PromptPreprocessResult:
    prompt: str
    visible_line: str = ""
    should_deliver: bool = True
    error_message: str = ""
    diagnostic_stage: str = ""
    diagnostic_code: str = ""
    recovery_action: str = ""


class PromptPreprocessor(Protocol):
    def __call__(
        self,
        prompt: str,
        target_thread_id: str | None = None,
    ) -> PromptPreprocessResult: ...


def keep_prompt(
    prompt: str,
    target_thread_id: str | None = None,
) -> PromptPreprocessResult:
    _ = target_thread_id
    return PromptPreprocessResult(prompt=prompt)


def block_prompt(diagnostic: ProRuntimeDiagnostic) -> PromptPreprocessResult:
    return PromptPreprocessResult(
        prompt="",
        visible_line="\n".join(
            (
                "!pro unavailable",
                f"Reason: {diagnostic.public_message}",
                f"Recovery: {diagnostic.recovery_action}",
                f"Code: {diagnostic.code.value}",
            )
        ),
        should_deliver=False,
        error_message=diagnostic.internal_detail,
        diagnostic_stage=diagnostic.stage.value,
        diagnostic_code=diagnostic.code.value,
        recovery_action=diagnostic.recovery_action,
    )


def ignore_discord_origin_prompt(target_thread_id: str | None, prompt: str) -> None:
    _ = target_thread_id, prompt


class PrepareMappedSessionMirrorOutput(Protocol[ChannelContraT]):
    def __call__(
        self, channel: ChannelContraT, target_thread_id: str | None
    ) -> Awaitable[bool]: ...


class ChannelTyping(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        *,
        context: str,
    ) -> AbstractAsyncContextManager[None]: ...


class TransportNoWait(Protocol):
    def __call__(
        self, prompt: str, target_thread_id: str | None
    ) -> Awaitable[tuple[int, str]]: ...


class ChunkSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        content: str,
        *,
        context: str | None = None,
    ) -> Awaitable[None]: ...


class AppMenuSender(Protocol[ChannelContraT]):
    def __call__(
        self,
        channel: ChannelContraT,
        target_thread_id: str | None,
        output: str,
        *,
        reason: str,
    ) -> Awaitable[bool]: ...


class ResumeFailureSender(Protocol[ChannelContraT]):
    def __call__(
        self, channel: ChannelContraT, content: str, target_thread_id: str
    ) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class MappedPromptDeliveryDeps(Generic[ChannelT]):
    prepare_mapped_session_mirror_output: PrepareMappedSessionMirrorOutput[ChannelT]
    set_selected_thread_id: SelectedThreadSetter
    channel_typing: ChannelTyping[ChannelT]
    preprocess_prompt: PromptPreprocessor
    mark_recent_discord_origin_prompt: DiscordOriginPromptMarker
    run_transport_prompt_no_wait: TransportNoWait
    send_chunks: ChunkSender[ChannelT]
    is_delivery_confirmation_timeout: OutputPredicate
    format_pending_ask_delivery_output: PendingFormatter
    release_session_mirror_output_target: OutputTargetReleaser
    is_selected_thread_busy_error: BusyPredicate
    send_codex_app_menu_if_available: AppMenuSender[ChannelT]
    send_resume_failure: ResumeFailureSender[ChannelT]
    format_log_text_len: TextLenFunc
    log: LogFunc


@dataclass(frozen=True, slots=True)
class MappedPromptDeliveryResult:
    handled: bool
    accepted: bool = False
    turn_id: str | None = None
    error_message: str = ""


def parse_transport_delivery_turn_id(output: str) -> str | None:
    prefixes = ("[app_server_delivery] turn_id=", "[ipc_delivery]")
    for line in output.splitlines():
        if line.startswith(prefixes[0]):
            return line.removeprefix(prefixes[0]).strip() or None
        if line.startswith(prefixes[1]):
            for field in line.split():
                if field.startswith("turn_id="):
                    return field.removeprefix("turn_id=").strip() or None
    return None


def format_mapped_transport_failure(exit_code: int, output: str) -> str:
    if output.strip() == "pro_chrome_unavailable":
        return "pro_chrome_unavailable"
    if "Prompt landed in a different thread" in output:
        return "Ask failed: Codex recorded this message in a different thread. I did not resend it here."
    return f"Ask failed (transport exit {exit_code})\n\n{output or '(no output)'}"


def is_thread_resume_timeout(output: str) -> bool:
    return "thread/resume" in output and "Timed out" in output


async def handle_mapped_prompt_delivery(
    channel: ChannelT,
    prompt: str,
    target_thread_id: str | None,
    *,
    deps: MappedPromptDeliveryDeps[ChannelT],
) -> MappedPromptDeliveryResult:
    if not await deps.prepare_mapped_session_mirror_output(channel, target_thread_id):
        return MappedPromptDeliveryResult(handled=False)
    if target_thread_id:
        deps.set_selected_thread_id(target_thread_id)
        deps.log(f"mapped_prompt_selected_thread_synced target={target_thread_id}")

    preprocessed = await asyncio.to_thread(
        deps.preprocess_prompt,
        prompt,
        target_thread_id,
    )
    if preprocessed.visible_line:
        await deps.send_chunks(
            channel, preprocessed.visible_line, context="prompt_preprocess_visible_line"
        )
    if not preprocessed.should_deliver:
        deps.log(
            "prompt_preprocess_blocked "
            + f"stage={preprocessed.diagnostic_stage or 'unspecified'} "
            + f"code={preprocessed.diagnostic_code or 'unspecified'} "
            + f"error={preprocessed.error_message or 'unspecified'}"
        )
        return MappedPromptDeliveryResult(
            handled=True,
            error_message=preprocessed.error_message,
        )
    if preprocessed.visible_line:
        deps.mark_recent_discord_origin_prompt(target_thread_id, preprocessed.prompt)

    async with deps.channel_typing(channel, context="ask_transport_no_wait"):
        exit_code, output = await deps.run_transport_prompt_no_wait(
            preprocessed.prompt, target_thread_id
        )
    turn_id = parse_transport_delivery_turn_id(output)
    deps.log(
        f"ask_transport_no_wait_done exit={exit_code} target={target_thread_id or '-'} "
        + f"output_len={deps.format_log_text_len(output)}"
    )
    if deps.is_delivery_confirmation_timeout(output):
        await deps.send_chunks(channel, deps.format_pending_ask_delivery_output(output))
        return MappedPromptDeliveryResult(
            handled=True,
            turn_id=turn_id,
            error_message="" if exit_code == 0 else output,
        )
    if exit_code == 0:
        return MappedPromptDeliveryResult(handled=True, accepted=True, turn_id=turn_id)
    released = await deps.release_session_mirror_output_target(target_thread_id)
    if not released:
        deps.log(
            "mapped_prompt_output_release_deferred "
            + f"target={target_thread_id or '-'} exit={exit_code}"
        )
    if deps.is_selected_thread_busy_error(
        exit_code, output
    ) and await deps.send_codex_app_menu_if_available(
        channel,
        target_thread_id,
        output,
        reason="ask_transport_no_wait_busy",
    ):
        return MappedPromptDeliveryResult(handled=True, error_message=output)
    failure = format_mapped_transport_failure(exit_code, output)
    if target_thread_id and is_thread_resume_timeout(output):
        await deps.send_resume_failure(channel, failure, target_thread_id)
    else:
        await deps.send_chunks(channel, failure)
    return MappedPromptDeliveryResult(handled=True, error_message=output)
