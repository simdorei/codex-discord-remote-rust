# pyright: reportAny=false, reportImplicitStringConcatenation=false
from __future__ import annotations

import argparse
from typing import Final

from codex_desktop_bridge_command_ask_types import CommandAskDeps, SessionOffsets
from codex_desktop_bridge_sidecar import CodexAppServerSidecar
from codex_pro_prompt_contract import is_pro_skill_prompt
from codex_thread_models import ThreadInfo


UI_DELIVERY_CONFIRM_TIMEOUT_SECONDS: Final = 4.0
PRO_UI_DELIVERY_CONFIRM_TIMEOUT_SECONDS: Final = 15.0


class CommandAskDeliveryError(RuntimeError):
    pass


def deliver_prompt(
    args: argparse.Namespace,
    deps: CommandAskDeps,
    thread: ThreadInfo,
    prompt: str,
    recent_offsets: SessionOffsets,
    *,
    use_sidecar: bool,
) -> CodexAppServerSidecar | None:
    if use_sidecar:
        return _deliver_via_sidecar(args, deps, thread, prompt, recent_offsets)
    if args.ipc:
        _deliver_via_ipc(args, deps, thread, prompt, recent_offsets)
    else:
        _deliver_via_ui(args, deps, thread, prompt, recent_offsets)
    return None


def _deliver_via_sidecar(
    args: argparse.Namespace,
    deps: CommandAskDeps,
    thread: ThreadInfo,
    prompt: str,
    recent_offsets: SessionOffsets,
) -> CodexAppServerSidecar | None:
    print("transport: local-sidecar turn/start")
    sidecar_result = deps.start_turn_via_sidecar(
        thread,
        prompt,
        timeout_sec=10.0,
        keep_client_open=not args.background,
    )
    maybe_client = sidecar_result.pop("_client", None)
    sidecar_client = maybe_client if isinstance(maybe_client, CodexAppServerSidecar) else None
    _print_delivery_confirmation(
        deps,
        recent_offsets,
        prompt,
        thread,
        timeout_sec=6.0,
        pending_message="Codex sidecar reported success, but local session recording was not confirmed before the deadline.",
        different_thread_prefix="Prompt landed in a different thread after sidecar delivery.",
    )
    print(
        f"[sidecar_delivery] turn_id={sidecar_result.get('turn_id') or '-'} "
        f"attempts={sidecar_result.get('attempts') or '-'}"
    )
    return sidecar_client


def _deliver_via_ipc(
    args: argparse.Namespace,
    deps: CommandAskDeps,
    thread: ThreadInfo,
    prompt: str,
    recent_offsets: SessionOffsets,
) -> None:
    print("ui_activation: ipc-thread-follower-start-turn")
    ipc_result = deps.start_turn_via_ipc(
        thread,
        prompt,
        timeout_sec=10.0,
        allow_ui_recovery=args.ipc_recover_ui,
    )
    _print_delivery_confirmation(
        deps,
        recent_offsets,
        prompt,
        thread,
        timeout_sec=6.0,
        pending_message="Codex IPC reported success, but local session recording was not confirmed before the deadline.",
        different_thread_prefix="Prompt landed in a different thread after IPC delivery.",
    )
    if ipc_result.get("recovery_method"):
        print(f"[ipc_recovery] {ipc_result['recovery_method']}")
    print(
        f"[ipc_delivery] owner_client={ipc_result['owner_client_id']} "
        f"turn_id={ipc_result['turn_id'] or '-'}"
    )


def _deliver_via_ui(
    args: argparse.Namespace,
    deps: CommandAskDeps,
    thread: ThreadInfo,
    prompt: str,
    recent_offsets: SessionOffsets,
) -> None:
    if args.switch_thread:
        activation_method = deps.activate_thread_in_ui(thread)
    else:
        verified_by = deps.verify_thread_in_ui(thread)
        if not verified_by:
            raise CommandAskDeliveryError(
                "The selected thread is not confirmed as the currently open Codex thread. "
                "Refusing to paste because it could create a new chat instead. "
                "Open the thread first or rerun with --switch-thread."
            )
        activation_method = f"already-open [{verified_by}]"
    print(f"ui_activation: {activation_method}")
    window = deps.send_prompt_to_codex(
        prompt=prompt,
        click_x_ratio=args.click_x_ratio,
        click_y_offset=args.click_y_offset,
        skip_click=not args.click,
    )
    print(
        deps.make_console_safe_text(
            f"sent_to_window: hwnd={window.hwnd} title={window.title} "
            f"rect=({window.left},{window.top})-({window.right},{window.bottom})"
        )
    )
    delivery_timeout = (
        PRO_UI_DELIVERY_CONFIRM_TIMEOUT_SECONDS
        if is_pro_skill_prompt(prompt)
        else UI_DELIVERY_CONFIRM_TIMEOUT_SECONDS
    )
    delivered_thread = deps.wait_for_prompt_delivery(recent_offsets, prompt, timeout_sec=delivery_timeout)
    if delivered_thread is None:
        raise CommandAskDeliveryError(
            "Prompt delivery could not be confirmed in any recent Codex thread. "
            "The UI likely moved, but the message was not recorded."
        )
    if delivered_thread.id != thread.id:
        raise CommandAskDeliveryError(
            "Prompt landed in a different thread. "
            f"Expected {deps.get_thread_label(thread)}, but it was recorded in {deps.get_thread_label(delivered_thread)}."
        )
    print(f"[delivery_verified] {deps.get_thread_label(thread)}")


def _print_delivery_confirmation(
    deps: CommandAskDeps,
    recent_offsets: SessionOffsets,
    prompt: str,
    thread: ThreadInfo,
    *,
    timeout_sec: float,
    pending_message: str,
    different_thread_prefix: str,
) -> None:
    delivered_thread = deps.wait_for_prompt_delivery(recent_offsets, prompt, timeout_sec=timeout_sec)
    if delivered_thread is None:
        print("[delivery_pending]")
        print(pending_message)
        print("Continuing to watch for the next Codex reply.")
        return
    if delivered_thread.id != thread.id:
        raise CommandAskDeliveryError(
            f"{different_thread_prefix} "
            f"Expected {deps.get_thread_label(thread)}, but it was recorded in {deps.get_thread_label(delivered_thread)}."
        )
    print(f"[delivery_verified] {deps.get_thread_label(thread)}")
