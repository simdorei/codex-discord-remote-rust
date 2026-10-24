$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
if ($args[0] -ne 'plugin') { throw 'Unexpected Codex command during installation' }
if ($args[1] -eq 'marketplace' -and $args[2] -eq 'add') { Write-Output 'Registered'; exit 0 }
if ($args[1] -eq 'add') { Write-Output 'Installed'; exit 0 }
$root = $env:CDR_RELEASE_TEST_ROOT
if ($args[1] -eq 'marketplace' -and $args[2] -eq 'list') {
    @{marketplaces=@(@{name='codex-discord-remote';root=$root})} | ConvertTo-Json -Depth 4 -Compress
    exit 0
}
if ($args[1] -eq 'list') {
    $manifest = Get-Content -LiteralPath (Join-Path $root 'plugins\codex-discord-remote\.codex-plugin\plugin.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    @{installed=@(@{pluginId='codex-discord-remote@codex-discord-remote';version=$manifest.version;installed=$true;enabled=$true})} |
        ConvertTo-Json -Depth 4 -Compress
    exit 0
}
throw 'Unexpected Codex plugin command'
