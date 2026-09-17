[CmdletBinding()]
param([string]$RepoRoot)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Join-Path $PSScriptRoot '..'
}
$RepoRoot = [IO.Path]::GetFullPath($RepoRoot)
Push-Location $RepoRoot
try {
    $metadataText = & cargo metadata --offline --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo workspace metadata.' }
    $metadata = $metadataText | ConvertFrom-Json
    $members = @($metadata.workspace_members)
    $packages = @($metadata.packages | Where-Object { $_.id -in $members })
    if ($members.Count -eq 0 -or $packages.Count -ne $members.Count) {
        throw 'Incomplete Cargo workspace metadata; formatting was not checked.'
    }
    $checked = 0
    $batches = 0
    foreach ($package in $packages) {
        $targets = @($package.targets | ForEach-Object {
            $edition = $_.edition
            if ([string]::IsNullOrWhiteSpace($edition)) { $edition = $package.edition }
            [pscustomobject]@{ source = $_.src_path; edition = $edition }
        } | Sort-Object edition,source -Unique)
        if ($targets.Count -eq 0) { throw "No formatting targets for $($package.name)." }
        foreach ($editionGroup in ($targets | Group-Object edition)) {
            $sources = @($editionGroup.Group.source)
            # Even a single large package can exceed Windows' command-line limit.
            # Keep all Cargo target roots and their editions; split only argv size.
            for ($offset = 0; $offset -lt $sources.Count; $offset += 8) {
                $last = [Math]::Min($offset + 7, $sources.Count - 1)
                $batch = @($sources[$offset..$last])
                & rustfmt --edition $editionGroup.Name --check @batch
                if ($LASTEXITCODE -ne 0) {
                    throw "Rust formatting check failed for $($package.name), edition $($editionGroup.Name), batch $batches."
                }
                $checked += $batch.Count
                $batches++
                Write-Output ([ordered]@{kind='rustfmt_batch';package=$package.name;edition=$editionGroup.Name;sources=$batch;exit_code=0}|ConvertTo-Json -Compress)
            }
        }
    }
    Write-Output "rustfmt_workspace_complete packages=$($packages.Count) target_roots=$checked batches=$batches"
} finally {
    Pop-Location
}
