# Puts assets\mdlite.ico into a built EXE. The toolchain here has no resource
# compiler, so the icon goes in afterwards through the documented resource
# update API rather than through a .rc file.
param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\dist\mdlite.exe'),
    [string]$Icon = (Join-Path $PSScriptRoot '..\assets\mdlite.ico')
)
$ErrorActionPreference = 'Stop'
Add-Type -Namespace MDLiteIcon -Name Native -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
public static extern IntPtr BeginUpdateResourceW(string file, bool deleteExisting);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern bool UpdateResourceW(IntPtr update, IntPtr type, IntPtr name, ushort language, byte[] data, uint size);
[DllImport("kernel32.dll", SetLastError = true)]
public static extern bool EndUpdateResourceW(IntPtr update, bool discard);
'@

$RT_ICON = [IntPtr]3
$RT_GROUP_ICON = [IntPtr]14
$NEUTRAL = [uint16]0

$bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Icon).Path)
if ([BitConverter]::ToUInt16($bytes, 2) -ne 1) { throw "$Icon is not an icon file." }
$count = [BitConverter]::ToUInt16($bytes, 4)

# The group directory the shell reads is the file's directory with each entry's
# offset replaced by the resource id the image was stored under.
$group = New-Object IO.MemoryStream
$writer = New-Object IO.BinaryWriter($group)
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$count)
$images = @()
for ($i = 0; $i -lt $count; $i++) {
    $entry = 6 + 16 * $i
    $size = [BitConverter]::ToUInt32($bytes, $entry + 8)
    $offset = [BitConverter]::ToUInt32($bytes, $entry + 12)
    $image = New-Object byte[] $size
    [Array]::Copy($bytes, $offset, $image, 0, $size)
    $images += , $image
    $writer.Write($bytes[$entry]); $writer.Write($bytes[$entry + 1])
    $writer.Write($bytes[$entry + 2]); $writer.Write($bytes[$entry + 3])
    $writer.Write([BitConverter]::ToUInt16($bytes, $entry + 4))
    $writer.Write([BitConverter]::ToUInt16($bytes, $entry + 6))
    $writer.Write([uint32]$size)
    $writer.Write([uint16]($i + 1))
}
$writer.Flush()

$target = (Resolve-Path -LiteralPath $Executable).Path
$update = [MDLiteIcon.Native]::BeginUpdateResourceW($target, $false)
if ($update -eq [IntPtr]::Zero) { throw "Could not open $target for resource update." }
try {
    for ($i = 0; $i -lt $count; $i++) {
        if (-not [MDLiteIcon.Native]::UpdateResourceW($update, $RT_ICON, [IntPtr]($i + 1),
                $NEUTRAL, $images[$i], $images[$i].Length)) {
            throw "Could not write icon $($i + 1)."
        }
    }
    $directory = $group.ToArray()
    # Group id 1: the lowest one is the icon Explorer shows for the file.
    if (-not [MDLiteIcon.Native]::UpdateResourceW($update, $RT_GROUP_ICON, [IntPtr]1,
            $NEUTRAL, $directory, $directory.Length)) {
        throw 'Could not write the icon directory.'
    }
}
catch { [void][MDLiteIcon.Native]::EndUpdateResourceW($update, $true); throw }
if (-not [MDLiteIcon.Native]::EndUpdateResourceW($update, $false)) {
    throw 'Could not commit the icon.'
}
"{0}  {1} icon sizes embedded" -f (Split-Path -Leaf $target), $count
