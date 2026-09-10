Set-StrictMode -Version Latest

function Publish-CdrCheckpointArchive {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$StagingRoot,
        [Parameter(Mandatory = $true)][string]$ArchivePath
    )
    $final = [IO.Path]::GetFullPath($ArchivePath)
    if ([IO.File]::Exists($final)) { throw "Checkpoint archive already exists: $final" }
    $parent = [IO.Path]::GetDirectoryName($final)
    $temporary = Join-Path $parent ".cdr-checkpoint-$([guid]::NewGuid().ToString('N')).tmp"
    try {
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        [IO.Compression.ZipFile]::CreateFromDirectory(
            $StagingRoot, $temporary, [IO.Compression.CompressionLevel]::Optimal, $false
        )
        if ([IO.File]::Exists($final)) { throw "Checkpoint archive already exists: $final" }
        [IO.File]::Move($temporary, $final)
        $temporary = $null
        return $final
    } finally {
        if ($null -ne $temporary -and [IO.File]::Exists($temporary)) {
            [IO.File]::Delete($temporary)
        }
    }
}

Export-ModuleMember -Function 'Publish-CdrCheckpointArchive'
