$ErrorActionPreference = 'Stop'
if ($env:GITHUB_REF -cnotmatch '^refs/tags/v\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$' -or
    $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$' -or -not $env:GITHUB_REPOSITORY) {
    throw 'Release publication requires a version tag and exact GitHub source commit.'
}
$deadline = [DateTimeOffset]::UtcNow.AddMinutes(75)
do {
    $response = & gh api "repos/$env:GITHUB_REPOSITORY/actions/runs?head_sha=$env:GITHUB_SHA&per_page=100"
    if ($LASTEXITCODE -ne 0) { throw 'GitHub check lookup failed; release was not published.' }
    $runs = ($response | ConvertFrom-Json).workflow_runs
    $ready = $true
    foreach ($name in @('Windows contract', 'macOS smoke')) {
        $run = $runs | Where-Object { $_.name -ceq $name -and $_.head_sha -ceq $env:GITHUB_SHA -and $_.event -eq 'push' } |
            Sort-Object run_number -Descending | Select-Object -First 1
        if (-not $run -or $run.status -ne 'completed') { $ready = $false; continue }
        if ($run.conclusion -ne 'success') { throw "Required check failed: $name ($($run.conclusion)). Release was not published." }
    }
    if ($ready) { break }
    if ([DateTimeOffset]::UtcNow -ge $deadline) { throw 'Timed out waiting for exact-commit checks; release was not published.' }
    Write-Output 'Waiting for Windows contract and macOS smoke checks on the release commit.'
    Start-Sleep -Seconds 30
} while ($true)
$version = $env:GITHUB_REF_NAME.Substring(1)
$name = "codex-discord-remote-$version-windows-x64.zip"
$archive = Join-Path $PSScriptRoot ('..\dist\' + $name)
$checksum = $archive + '.sha256'
if (-not (Test-Path -LiteralPath $archive) -or -not (Test-Path -LiteralPath $checksum)) { throw 'Release assets are missing.' }
$notes = Join-Path $env:RUNNER_TEMP 'cdr-release-notes.md'
$text = @"
Windows x64 download: **$name**. Download the ZIP asset, extract all files and run ``install.cmd``.

Git, Rust and Visual Studio are not required. Install Codex separately. Installation does not start the bot or register automatic startup; run ``setup-discord-bot.ps1`` separately when ready.

The ZIP includes the bot, required helper, plugin, scripts and setup guide. SHA-256 checksums are attached. GitHub's automatic **Source code** archives do not contain executables.

Source commit: $env:GITHUB_SHA. Windows contract, macOS smoke and extracted release installation checks passed.
"@
[IO.File]::WriteAllText($notes, $text, [Text.UTF8Encoding]::new($false))
& gh release create $env:GITHUB_REF_NAME $archive $checksum --repo $env:GITHUB_REPOSITORY --verify-tag `
    --title "Codex Discord Remote $version - Windows x64" --notes-file $notes
if ($LASTEXITCODE -ne 0) { throw 'GitHub release publication failed.' }
