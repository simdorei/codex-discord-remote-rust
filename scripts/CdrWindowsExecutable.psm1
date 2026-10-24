# Read the PE import table without requiring Visual Studio on the verification machine.
function Test-CdrWindowsExecutable {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Path)
    $ErrorActionPreference = 'Stop'
    $stream = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) { throw "Not a Windows executable: $Path" }
        $stream.Position = 0x3c
        $pe = $reader.ReadUInt32()
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "Expected an AMD64 Windows executable: $Path"
        }
        $sectionCount = $reader.ReadUInt16()
        $stream.Position = $pe + 20
        $optionalSize = $reader.ReadUInt16()
        $optional = $pe + 24
        $stream.Position = $optional
        if ($reader.ReadUInt16() -ne 0x20b) { throw "Expected PE32+ executable: $Path" }
        $stream.Position = $optional + 120
        $importRva = $reader.ReadUInt32()
        $sections = @()
        for ($i = 0; $i -lt $sectionCount; $i++) {
            $stream.Position = $optional + $optionalSize + $i * 40 + 8
            $virtualSize = $reader.ReadUInt32(); $rva = $reader.ReadUInt32()
            $rawSize = $reader.ReadUInt32(); $offset = $reader.ReadUInt32()
            $sections += @{rva=$rva; size=[Math]::Max($virtualSize,$rawSize); offset=$offset}
        }
        function Resolve-FileOffset([uint32]$Rva) {
            foreach ($section in $sections) {
                if ($Rva -ge $section.rva -and $Rva -lt $section.rva + $section.size) {
                    return [long]($Rva - $section.rva + $section.offset)
                }
            }
            throw "Invalid import address in executable: $Path"
        }
        $imports = [Collections.Generic.List[string]]::new()
        if ($importRva -eq 0) { throw "Executable has no Windows imports: $Path" }
        $table = Resolve-FileOffset $importRva
        for ($i = 0; $i -lt 512; $i++) {
            $stream.Position = $table + $i * 20 + 12
            $nameRva = $reader.ReadUInt32()
            if ($nameRva -eq 0) { break }
            $stream.Position = Resolve-FileOffset $nameRva
            $name = [Text.StringBuilder]::new()
            for ($j = 0; $j -lt 256; $j++) {
                $byte = $reader.ReadByte()
                if ($byte -eq 0) { break }
                $null = $name.Append([char]$byte)
            }
            $dll = $name.ToString()
            # Windows API-set contracts are resolved by the loader; many have no
            # physical DLL with the contract name in System32.
            $apiSet = $dll -match '^(api|ext)-ms-win-[a-z0-9-]+\.dll$'
            if ($dll -match '^(vcruntime|msvcp|concrt|libgcc|libstdc\+\+|libwinpthread)' -or
                $dll -match '[\\/:]' -or (-not $apiSet -and -not [IO.File]::Exists((Join-Path $env:SystemRoot ('System32\' + $dll))))) {
                throw "Executable requires an unbundled runtime library: $dll ($Path)"
            }
            $imports.Add($dll)
        }
        if ($i -eq 512) { throw "Invalid unterminated import table: $Path" }
        Write-Output "Verified Windows x64 executable with system libraries only: $([IO.Path]::GetFileName($Path))"
    } finally { $reader.Dispose(); $stream.Dispose() }
}

Export-ModuleMember -Function Test-CdrWindowsExecutable
