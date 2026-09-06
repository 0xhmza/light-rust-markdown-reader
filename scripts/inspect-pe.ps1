param([string]$Executable = (Join-Path $PSScriptRoot '..\dist\mdlite.exe'))
$ErrorActionPreference = 'Stop'
$data = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Executable).Path)
function U16([int]$Offset) { [BitConverter]::ToUInt16($data, $Offset) }
function U32([int]$Offset) { [BitConverter]::ToUInt32($data, $Offset) }
$pe = U32 0x3c
if ((U32 $pe) -ne 0x4550) { throw 'Not a PE executable.' }
$machine = U16 ($pe + 4)
$sections = U16 ($pe + 6)
$optionalSize = U16 ($pe + 20)
$optional = $pe + 24
if ((U16 $optional) -ne 0x20b) { throw 'Expected a 64-bit executable.' }
$sectionStart = $optional + $optionalSize
function FileOffset([uint32]$Rva) {
    for ($i = 0; $i -lt $sections; $i++) {
        $entry = $sectionStart + $i * 40
        $virtualSize = U32 ($entry + 8)
        $virtualAddress = U32 ($entry + 12)
        $rawSize = U32 ($entry + 16)
        $rawOffset = U32 ($entry + 20)
        if ($Rva -ge $virtualAddress -and $Rva -lt ($virtualAddress + [math]::Max($virtualSize, $rawSize))) {
            return $rawOffset + $Rva - $virtualAddress
        }
    }
    throw "Unmapped RVA: $Rva"
}
$imports = [Collections.Generic.List[string]]::new()
$importRva = U32 ($optional + 120)
if ($importRva) {
    $cursor = FileOffset $importRva
    while ((U32 $cursor) -or (U32 ($cursor + 12)) -or (U32 ($cursor + 16))) {
        $start = FileOffset (U32 ($cursor + 12))
        $end = $start
        while ($data[$end]) { $end++ }
        $imports.Add([Text.Encoding]::ASCII.GetString($data, $start, ($end - $start)))
        $cursor += 20
    }
}
[pscustomobject]@{
    File = (Resolve-Path -LiteralPath $Executable).Path
    Bytes = $data.Length
    KiB = [math]::Round($data.Length / 1KB, 1)
    Machine = ('0x{0:X}' -f $machine)
    Subsystem = U16 ($optional + 68)
    ImportedDlls = $imports.ToArray()
    SHA256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
} | ConvertTo-Json
