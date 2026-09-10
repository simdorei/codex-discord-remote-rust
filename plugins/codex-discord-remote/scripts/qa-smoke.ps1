[CmdletBinding()]
param(
    [string]$RepoRoot,
    [switch]$SkipUnitTests
)

$ErrorActionPreference = 'Stop'

function Invoke-NativeChecked {
    param(
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command
    )

    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Command
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
    if ($exitCode -ne 0) {
        throw "Native command failed with exit code $exitCode."
    }
}

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Join-Path $PSScriptRoot '..\..\..'
}

$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)

Push-Location $RepoRoot
try {
    Invoke-NativeChecked { git diff --check }

    Invoke-NativeChecked { powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -DryRun -SkipDependencies -SkipEnvFile }

    $GitBash = $env:OMO_CODEX_GIT_BASH_PATH
    if ([string]::IsNullOrWhiteSpace($GitBash)) {
        $DefaultGitBash = 'C:\Program Files\Git\bin\bash.exe'
        if (Test-Path -LiteralPath $DefaultGitBash) {
            $GitBash = $DefaultGitBash
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($GitBash)) {
        Invoke-NativeChecked { & $GitBash -lc 'bash -n install.sh setup-discord-bot.sh codex-discord-bot.sh && bash install.sh --dry-run --skip-dependencies --skip-env-file --skip-codex-plugin && bash setup-discord-bot.sh --dry-run' }
    }

    Invoke-NativeChecked { & py -3 -m py_compile `
        setup_discord_bot.py `
        codex_app_server_transport.py `
        codex_desktop_bridge_active_thread.py `
        codex_desktop_bridge_busy_state.py `
        codex_desktop_bridge.py `
        codex_desktop_bridge_command_ask.py `
        codex_desktop_bridge_command_ask_types.py `
        codex_desktop_bridge_final_answer.py `
        codex_desktop_bridge_cli.py `
        codex_desktop_bridge_cli_ask.py `
        codex_desktop_bridge_desktop_process.py `
        codex_desktop_bridge_desktop_resolver.py `
        codex_desktop_bridge_ipc_start_turn.py `
        codex_desktop_bridge_ipc_submit.py `
        codex_desktop_bridge_interactive_session.py `
        codex_desktop_bridge_macos_input.py `
        codex_desktop_bridge_macos_ui.py `
        codex_desktop_bridge_pending.py `
        codex_desktop_bridge_permission_ui.py `
        codex_desktop_bridge_reply.py `
        codex_desktop_bridge_reply_payload.py `
        codex_desktop_bridge_session_files.py `
        codex_desktop_bridge_sidebar_activation.py `
        codex_desktop_bridge_sidecar.py `
        codex_desktop_bridge_sidecar_process.py `
        codex_desktop_bridge_sidecar_resolver.py `
        codex_desktop_bridge_thread_activation.py `
        codex_desktop_bridge_unsupported_input.py `
        codex_desktop_bridge_window_focus.py `
        codex_discord_button_qa_cases.py `
        codex_discord_button_qa_lifecycle_cases.py `
        codex_discord_button_qa_persistent_cases.py `
        codex_discord_button_qa_steer_case.py `
        codex_discord_attachments.py `
        codex_discord_busy.py `
        codex_discord_busy_choice_queue_action.py `
        codex_discord_busy_choice_steer_failure.py `
        codex_discord_busy_choice_steer_result.py `
        codex_discord_bot.py `
        codex_discord_component_view_state.py `
        codex_discord_commands.py `
        codex_discord_delivery.py `
        codex_discord_delivery_state.py `
        codex_discord_help.py `
        codex_discord_interaction_gate.py `
        codex_discord_message_gate.py `
        codex_discord_mirror_channels.py `
        codex_discord_mirror_status.py `
        codex_discord_mirror_thread_channels.py `
        codex_discord_persistent_busy_choice.py `
        codex_discord_persistent_busy_queue.py `
        codex_discord_persistent_busy_steer.py `
        codex_discord_persistent_busy_steer_action.py `
        codex_discord_persistent_busy_steer_result.py `
        codex_discord_persistent_interactions.py `
        codex_discord_prefix_approval_commands.py `
        codex_discord_prefix_archive_commands.py `
        codex_discord_prefix_mirror_commands.py `
        codex_discord_prefix_new_command.py `
        codex_discord_prefix_dispatch.py `
        codex_discord_prefix_prompt_commands.py `
        codex_discord_prefix_qa_command.py `
        codex_discord_prefix_queue_commands.py `
        codex_discord_prefix_steer_command.py `
        codex_discord_prefix_status_commands.py `
        codex_discord_prompt_busy_result.py `
        codex_discord_prompt_flow.py `
        codex_discord_prompt_mapped_delivery.py `
        codex_discord_prompt_pending_delivery.py `
        codex_discord_prompt_retry_attempt.py `
        codex_discord_prompt_retry_exhausted.py `
        codex_discord_prompt_retry_suppression.py `
        codex_discord_prompt_stream_attempt.py `
        codex_discord_prompt_stream_result.py `
        codex_discord_prompt_stream_suppression.py `
        codex_discord_prompt_transport.py `
        codex_discord_projects.py `
        codex_discord_runner.py `
        codex_discord_runtime.py `
        codex_discord_session_mirror.py `
        codex_discord_session_mirror_archive.py `
        codex_discord_session_mirror_channels.py `
        codex_discord_session_mirror_commit.py `
        codex_discord_session_mirror_cursor.py `
        codex_discord_session_mirror_event_reader.py `
        codex_discord_session_mirror_item_delivery.py `
        codex_discord_session_mirror_item_sender.py `
        codex_discord_session_mirror_items.py `
        codex_discord_session_mirror_output_targets.py `
        codex_discord_slash_commands.py `
        codex_discord_slash_prompt_commands.py `
        codex_discord_slash_runtime_commands.py `
        codex_discord_stale_busy_components.py `
        codex_discord_steering.py `
        codex_discord_store.py `
        codex_discord_thread_state.py `
        tests\test_codex_discord_component_view_state.py `
        tests\test_codex_desktop_bridge_active_thread.py `
        tests\test_codex_desktop_bridge_busy_state.py `
        tests\codex_desktop_bridge_command_ask_fakes.py `
        tests\test_codex_desktop_bridge_command_ask.py `
        tests\test_codex_desktop_bridge_command_ask_edge.py `
        tests\test_codex_desktop_bridge_final_answer.py `
        tests\test_codex_desktop_bridge_list_settings.py `
        tests\test_codex_desktop_bridge_cli.py `
        tests\test_codex_desktop_bridge_desktop_process.py `
        tests\test_codex_desktop_bridge_ipc_start_turn.py `
        tests\test_codex_desktop_bridge_ipc_submit.py `
        tests\test_codex_desktop_bridge_interactive_session.py `
        tests\test_codex_desktop_bridge_macos_input.py `
        tests\test_codex_desktop_bridge_pending.py `
        tests\test_codex_desktop_bridge_permission_ui.py `
        tests\test_codex_desktop_bridge_reply.py `
        tests\test_codex_desktop_bridge_reply_payload.py `
        tests\test_codex_desktop_bridge_sidebar_activation.py `
        tests\test_codex_desktop_bridge_sidecar.py `
        tests\test_codex_desktop_bridge_thread_activation.py `
        tests\test_codex_desktop_bridge_window_focus.py `
        tests\test_codex_discord_button_qa_cases.py `
        tests\test_codex_discord_button_qa_lifecycle_cases.py `
        tests\test_codex_discord_button_qa_persistent_cases.py `
        tests\test_codex_discord_button_qa_steer_case.py `
        tests\test_codex_discord_busy.py `
        tests\test_codex_discord_busy_choice_queue_action.py `
        tests\test_codex_discord_busy_choice_steer_failure.py `
        tests\test_codex_discord_busy_choice_steer_result.py `
        tests\test_codex_discord_delivery.py `
        tests\test_codex_discord_interaction_gate.py `
        tests\test_codex_discord_message_intake.py `
        tests\test_codex_discord_mirror_channels.py `
        tests\test_codex_discord_persistent_busy_choice.py `
        tests\test_codex_discord_persistent_busy_choice_queue.py `
        tests\test_codex_discord_persistent_busy_queue.py `
        tests\test_codex_discord_persistent_busy_choice_steer.py `
        tests\test_codex_discord_persistent_busy_steer_action.py `
        tests\test_codex_discord_persistent_busy_steer_busy_failure.py `
        tests\test_codex_discord_persistent_busy_steer_result.py `
        tests\test_codex_discord_persistent_busy_steer_runner.py `
        tests\test_codex_discord_persistent_interactions.py `
        tests\test_codex_discord_prefix_approval_commands.py `
        tests\test_codex_discord_prefix_archive_commands.py `
        tests\test_codex_discord_prefix_mirror_commands.py `
        tests\test_codex_discord_prefix_new_command.py `
        tests\test_codex_discord_prefix_dispatch.py `
        tests\test_codex_discord_prefix_prompt_commands.py `
        tests\test_codex_discord_prefix_qa_command.py `
        tests\test_codex_discord_prefix_queue_commands.py `
        tests\test_codex_discord_prefix_steer_command.py `
        tests\test_codex_discord_prefix_status_commands.py `
        tests\test_codex_discord_prompt_busy_result.py `
        tests\test_codex_discord_prompt_flow.py `
        tests\test_codex_discord_prompt_mapped_delivery.py `
        tests\test_codex_discord_prompt_pending_delivery.py `
        tests\test_codex_discord_prompt_retry_attempt.py `
        tests\test_codex_discord_prompt_retry_exhausted.py `
        tests\test_codex_discord_prompt_retry_suppression.py `
        tests\test_codex_discord_prompt_stream_attempt.py `
        tests\test_codex_discord_prompt_stream_result.py `
        tests\test_codex_discord_prompt_stream_suppression.py `
        tests\test_codex_discord_prompt_transport.py `
        tests\test_codex_discord_session_mirror_archive.py `
        tests\test_codex_discord_session_mirror_channels.py `
        tests\test_codex_discord_session_mirror_commit.py `
        tests\test_codex_discord_session_mirror_cursor.py `
        tests\test_codex_discord_session_mirror_event_reader.py `
        tests\test_codex_discord_session_mirror_item_delivery.py `
        tests\test_codex_discord_session_mirror_item_sender.py `
        tests\test_codex_discord_session_mirror_items.py `
        tests\test_codex_discord_session_mirror_output_targets.py `
        tests\test_codex_discord_slash_commands.py `
        tests\test_codex_discord_slash_prompt_commands.py `
        tests\test_codex_discord_slash_runtime_commands.py `
        tests\test_codex_discord_stale_busy_components.py `
        tests\test_codex_discord_bot.py `
        tests\test_mirror_sync_cleanup.py `
        tests\test_setup_discord_bot.py }

    if (-not $SkipUnitTests) {
        Invoke-NativeChecked { & py -3 -m unittest tests.test_setup_discord_bot tests.test_codex_desktop_bridge_macos_input tests.test_codex_discord_button_qa_cases tests.test_codex_discord_button_qa_lifecycle_cases tests.test_codex_discord_button_qa_persistent_cases tests.test_codex_discord_button_qa_steer_case tests.test_codex_discord_busy tests.test_codex_discord_busy_choice_queue_action tests.test_codex_discord_busy_choice_steer_failure tests.test_codex_discord_busy_choice_steer_result tests.test_codex_discord_component_view_state tests.test_codex_desktop_bridge_command_ask tests.test_codex_desktop_bridge_command_ask_edge tests.test_codex_desktop_bridge_final_answer tests.test_codex_desktop_bridge_list_settings tests.test_codex_discord_delivery tests.test_codex_discord_persistent_busy_choice tests.test_codex_discord_persistent_busy_choice_queue tests.test_codex_discord_persistent_busy_queue tests.test_codex_discord_persistent_busy_choice_steer tests.test_codex_discord_persistent_busy_steer_action tests.test_codex_discord_persistent_busy_steer_busy_failure tests.test_codex_discord_persistent_busy_steer_result tests.test_codex_discord_persistent_busy_steer_runner tests.test_codex_discord_persistent_interactions tests.test_codex_discord_prefix_approval_commands tests.test_codex_discord_prefix_archive_commands tests.test_codex_discord_prefix_mirror_commands tests.test_codex_discord_prefix_new_command tests.test_codex_discord_prefix_dispatch tests.test_codex_discord_prefix_prompt_commands tests.test_codex_discord_prefix_qa_command tests.test_codex_discord_prefix_queue_commands tests.test_codex_discord_prefix_steer_command tests.test_codex_discord_prefix_status_commands tests.test_codex_discord_prompt_busy_result tests.test_codex_discord_prompt_flow tests.test_codex_discord_prompt_mapped_delivery tests.test_codex_discord_prompt_pending_delivery tests.test_codex_discord_prompt_retry_attempt tests.test_codex_discord_prompt_retry_exhausted tests.test_codex_discord_prompt_retry_suppression tests.test_codex_discord_prompt_stream_attempt tests.test_codex_discord_prompt_stream_result tests.test_codex_discord_prompt_stream_suppression tests.test_codex_discord_prompt_transport tests.test_codex_discord_session_mirror_archive tests.test_codex_discord_session_mirror_channels tests.test_codex_discord_session_mirror_commit tests.test_codex_discord_session_mirror_cursor tests.test_codex_discord_session_mirror_event_reader tests.test_codex_discord_session_mirror_item_delivery tests.test_codex_discord_session_mirror_item_sender tests.test_codex_discord_session_mirror_items tests.test_codex_discord_session_mirror_output_targets tests.test_codex_discord_slash_commands tests.test_codex_discord_slash_prompt_commands tests.test_codex_discord_slash_runtime_commands tests.test_codex_discord_stale_busy_components tests.test_codex_discord_mirror_channels tests.test_codex_discord_bot tests.test_mirror_sync_cleanup }
    }
} finally {
    Pop-Location
}
