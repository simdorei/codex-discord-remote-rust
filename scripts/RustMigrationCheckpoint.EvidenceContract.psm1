Import-Module (Join-Path $PSScriptRoot 'RustMigrationCheckpoint.EvidenceShape.psm1') -ErrorAction Stop
Set-StrictMode -Version Latest

function Assert-CdrOfflineSoakEvidenceContract {
    param([AllowNull()][object]$Evidence)
    try {
        Assert-CdrOfflineSoakEvidenceShape $Evidence
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceInteger $Evidence.schema_version) -and
                [uint64]$Evidence.schema_version -eq 1) 'schema_version must be integer 1'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString $Evidence.kind `
            'windows_offline_fake_replay_soak_smoke') 'kind is invalid'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString $Evidence.status `
            'passed_short_smoke_only') 'status is invalid'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.operational_status 'passed') 'operational_status must be passed'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.harness.status 'passed') 'harness.status must be passed'
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceInteger $Evidence.harness.cycles) -and
                [uint64]$Evidence.harness.cycles -gt 0) 'harness.cycles must be positive'
        foreach ($pair in @(
            @('queue_completed', 'queue_submitted'), @('outbox_delivered', 'outbox_staged')
        )) {
            $actual = $Evidence.harness.($pair[0]); $expected = $Evidence.harness.($pair[1])
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $actual) -and (Test-CdrEvidenceInteger $expected) -and
                    [uint64]$actual -eq [uint64]$expected -and [uint64]$expected -gt 0) `
                "harness.$($pair[0]) must equal positive harness.$($pair[1])"
        }
        foreach ($name in @('duplicate_successes', 'target_stalls', 'queue_remaining', 'outbox_remaining')) {
            $value = $Evidence.harness.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -eq 0) `
                "harness.$name must be zero"
        }
        foreach ($name in @('mirror_send_failures', 'mirror_send_retries', 'exact_message_retries')) {
            $value = $Evidence.harness.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -gt 0) `
                "harness.$name must prove the retry path"
        }
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceInteger $Evidence.memory.sample_count) -and
                [uint64]$Evidence.memory.sample_count -ge 2) 'memory.sample_count must be at least 2'
        Assert-CdrEvidenceCondition `
            ($Evidence.memory.threshold_passed -is [bool] -and $Evidence.memory.threshold_passed) `
            'memory.threshold_passed must be true'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.memory.interpretation `
            'startup_plumbing_only_not_a_stability_or_production_memory_no_regression_result') `
            'memory interpretation must not claim stability'
        foreach ($name in @('cdr_runtime', 'cdr_offline_soak', 'cdr_mcp_server')) {
            Assert-CdrEvidenceArtifact $Evidence.artifacts.$name "artifacts.$name"
        }
        foreach ($name in @(
            'same_run_canonical_release_build', 'canonical_release_harness',
            'build_before_after_equal', 'soak_before_after_equal',
            'harness_hash_expected_before_after_equal', 'harness_provenance_verified'
        )) {
            Assert-CdrEvidenceCondition `
                ($Evidence.provenance.$name -is [bool] -and $Evidence.provenance.$name) `
                "provenance.$name must be true"
        }
        Assert-CdrEvidenceHash $Evidence.provenance.source_fingerprint `
            'provenance.source_fingerprint'
        foreach ($name in @('source_file_count', 'source_total_bytes')) {
            $value = $Evidence.provenance.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -gt 0) `
                "provenance.$name must be a positive integer"
        }
        Assert-CdrEvidenceHash $Evidence.provenance.local_summary_sha256 `
            'provenance.local_summary_sha256'
        foreach ($name in @(
            'bot_disabled_before_during_after', 'operator_disabled_marker_preserved', 'cleanup_clean'
        )) {
            Assert-CdrEvidenceCondition `
                ($Evidence.safety.$name -is [bool] -and $Evidence.safety.$name) `
                "safety.$name must be true"
        }
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceInteger $Evidence.safety.child_exit_code) -and
                [uint64]$Evidence.safety.child_exit_code -eq 0) `
            'safety.child_exit_code must be zero'
        foreach ($name in @('discord_or_network_boundary_invoked', 'secrets_in_evidence')) {
            Assert-CdrEvidenceCondition `
                ($Evidence.safety.$name -is [bool] -and -not $Evidence.safety.$name) `
                "safety.$name must be false"
        }
        foreach ($name in @(
            'production_runtime_process_count_after', 'offline_soak_process_count_after',
            'mcp_server_process_count_after'
        )) {
            $value = $Evidence.safety.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -eq 0) `
                "safety.$name must be zero"
        }
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceExactString $Evidence.final_eligibility.status 'ineligible') -and
                $Evidence.final_eligibility.eligible -is [bool] -and
                -not $Evidence.final_eligibility.eligible -and
                $Evidence.final_eligibility.expected_for_short_smoke -is [bool] -and
                $Evidence.final_eligibility.expected_for_short_smoke) `
            'final_eligibility must identify an expected short-smoke ineligible result'
        Assert-CdrEvidenceCondition `
            ($Evidence.long_run.completed -is [bool] -and -not $Evidence.long_run.completed -and
                (Test-CdrEvidenceExactString $Evidence.long_run.status 'pending')) `
            'long_run must remain explicitly pending'
    } catch {
        throw "Offline soak evidence contract failed: $($_.Exception.Message)"
    }
}

function Assert-CdrWorkspaceGateEvidenceContract {
    param([AllowNull()][object]$Evidence)
    try {
        Assert-CdrWorkspaceGateEvidenceShape $Evidence
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceInteger $Evidence.schema_version) -and
                [uint64]$Evidence.schema_version -eq 1) 'schema_version must be integer 1'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.kind 'windows_full_workspace_gate') 'kind is invalid'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.status 'passed') 'status must be passed'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.platform 'windows') 'platform must be windows'
        Assert-CdrEvidenceHash $Evidence.source_fingerprint 'source_fingerprint'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.source_scope.schema 'cdr.rust-source-fingerprint.v1') `
            'source_scope.schema is invalid'
        foreach ($name in @('file_count', 'total_bytes')) {
            $value = $Evidence.source_scope.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -gt 0) `
                "source_scope.$name must be a positive integer"
        }
        foreach ($name in @(
            'cargo_test_workspace_all_targets_locked_offline_no_fail_fast',
            'cargo_clippy_workspace_all_targets_locked_offline_deny_warnings',
            'cargo_fmt_all_check', 'cargo_doc_workspace_no_deps_locked_offline_deny_warnings',
            'cargo_build_workspace_release_locked_offline'
        )) {
            Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
                $Evidence.rust.$name 'passed_exit_0') `
                "rust.$name must be passed_exit_0"
        }
        foreach ($name in @(
            'powershell_5_1_source_fingerprint_evidence_contract',
            'powershell_7_6_source_fingerprint_evidence_contract'
        )) {
            Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
                $Evidence.rust.$name 'passed') `
                "rust.$name must be passed"
        }
        $checkpoint = $Evidence.rust.release_checkpoint_contracts
        $expected = [ordered]@{
            base = 16; evidence_integrity = 14; rollback_completeness = 6
            staged_binding = 5; archive_adversarial = 1; failed = 0
        }
        foreach ($name in $expected.Keys) {
            $value = $checkpoint.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and
                    [uint64]$value -eq [uint64]$expected[$name]) `
                "rust.release_checkpoint_contracts.$name is not the current passing count"
        }
        foreach ($name in @(
            'pytest_full_suite', 'pro_plugin_contract_suite', 'installer_unittests',
            'desktop_bridge_tests', 'durable_store_tests'
        )) {
            $result = $Evidence.python.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $result.passed) -and [uint64]$result.passed -gt 0 -and
                    (Test-CdrEvidenceInteger $result.failed) -and [uint64]$result.failed -eq 0) `
                "python.$name must contain positive passed and zero failed counts"
        }
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.python.workflow_py_compile 'passed') `
            'python.workflow_py_compile must be passed'
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.python.install_ps1_dry_run_skip_dependencies_env_plugin 'passed') `
            'python.install_ps1_dry_run_skip_dependencies_env_plugin must be passed'
        foreach ($name in @(
            'rust_utf8_bom_count', 'rust_invalid_utf8_count',
            'production_rust_files_over_250_lines', 'changed_text_utf8_bom_count',
            'changed_text_invalid_utf8_count'
        )) {
            $value = $Evidence.quality.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -eq 0) `
                "quality.$name must be zero"
        }
        Assert-CdrEvidenceCondition `
            ((Test-CdrEvidenceInteger $Evidence.quality.rust_files_checked) -and
                [uint64]$Evidence.quality.rust_files_checked -gt 0) `
            'quality.rust_files_checked must be a positive integer'
        foreach ($name in @('cdr_runtime', 'cdr_offline_soak', 'cdr_mcp_server')) {
            Assert-CdrEvidenceArtifact $Evidence.artifacts.$name "artifacts.$name"
            Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
                $Evidence.artifacts.$name.pe_magic 'MZ') `
                "artifacts.$name.pe_magic must be MZ"
        }
        Assert-CdrEvidenceCondition `
            ($Evidence.artifacts.repository_target_release_matches_external_build -is [bool] -and
                $Evidence.artifacts.repository_target_release_matches_external_build) `
            'repository_target_release_matches_external_build must be true'
        foreach ($name in @('bot_disabled', 'disabled_marker_present')) {
            Assert-CdrEvidenceCondition `
                ($Evidence.safety.$name -is [bool] -and $Evidence.safety.$name) `
                "safety.$name must be true"
        }
        Assert-CdrEvidenceCondition (Test-CdrEvidenceExactString `
            $Evidence.safety.runtime_mode 'rust') `
            'safety.runtime_mode must be rust'
        foreach ($name in @(
            'runtime_process_count_after', 'offline_soak_process_count_after',
            'mcp_server_process_count_after'
        )) {
            $value = $Evidence.safety.$name
            Assert-CdrEvidenceCondition `
                ((Test-CdrEvidenceInteger $value) -and [uint64]$value -eq 0) `
                "safety.$name must be zero"
        }
        foreach ($name in @('live_discord_or_network_smoke_run', 'secrets_in_evidence')) {
            Assert-CdrEvidenceCondition `
                ($Evidence.safety.$name -is [bool] -and -not $Evidence.safety.$name) `
                "safety.$name must be false"
        }
        Assert-CdrEvidenceCondition ($null -ne $Evidence.powershell_source) `
            'powershell_source is required'
        Assert-CdrEvidenceCondition ($null -ne $Evidence.rollback_source) `
            'rollback_source is required'
    } catch {
        throw "Workspace gate evidence contract failed: $($_.Exception.Message)"
    }
}

Export-ModuleMember -Function @(
    'Assert-CdrOfflineSoakEvidenceContract', 'Assert-CdrWorkspaceGateEvidenceContract'
)
