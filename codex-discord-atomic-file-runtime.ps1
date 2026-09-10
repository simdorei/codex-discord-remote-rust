function Move-AtomicTextFile {
    param(
        [string]$Source,
        [string]$Destination
    )

    [System.IO.File]::Move($Source, $Destination)
}

function Publish-AtomicTextFile {
    param(
        [string]$Path,
        [string]$Content
    )

    $directory = [System.IO.Path]::GetDirectoryName($Path)
    $name = [System.IO.Path]::GetFileName($Path)
    $temporaryPath = Join-Path `
        $directory `
        ('.' + $name + '.' + [Guid]::NewGuid().ToString('N') + '.tmp')
    try {
        [System.IO.File]::WriteAllText(
            $temporaryPath,
            $Content,
            [System.Text.UTF8Encoding]::new($false)
        )
        $lastCollision = $null
        for ($attempt = 0; $attempt -lt 32; $attempt++) {
            try {
                Move-AtomicTextFile `
                    -Source $temporaryPath `
                    -Destination $Path
                return
            } catch [System.IO.IOException] {
                $nativeError = $_.Exception.HResult -band 0xFFFF
                if ($nativeError -notin @(80, 183)) {
                    throw
                }
                $lastCollision = $_.Exception
            }
            try {
                $publishedContent = [System.IO.File]::ReadAllText(
                    $Path,
                    [System.Text.UTF8Encoding]::new($false)
                )
            } catch [System.IO.FileNotFoundException] {
                continue
            }
            if ([string]::Equals(
                $publishedContent,
                $Content,
                [StringComparison]::Ordinal
            )) {
                return
            }
            throw $lastCollision
        }
        throw $lastCollision
    } finally {
        Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
    }
}
