param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\dist\mdlite.exe'),
    [string]$Report = 'smoke-result.json'
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class MDLiteSmoke {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Scroll { public uint Size, Mask; public int Min, Max; public uint Page; public int Pos, Track; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int count);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hwnd, StringBuilder name, int count);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, UIntPtr wp, IntPtr lp, uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool GetScrollInfo(IntPtr hwnd, int bar, ref Scroll info);
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    public static IntPtr Find(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, param) => { uint pid; GetWindowThreadProcessId(hwnd, out pid); if (pid == processId) { var name = new StringBuilder(256); GetClassNameW(hwnd, name, name.Capacity); if (name.ToString() == "MDLite.NativeReader") { found = hwnd; return false; } } return true; }, IntPtr.Zero);
        return found;
    }
    public static void Send(IntPtr hwnd, uint message, long wp = 0) {
        UIntPtr result;
        if (SendMessageTimeoutW(hwnd, message, unchecked((UIntPtr)(ulong)wp), IntPtr.Zero, 2, 30000, out result) == IntPtr.Zero) throw new Exception("Window did not respond: " + message);
    }
}
'@
$qa = Join-Path $PSScriptRoot '..\.tools\qa'
New-Item -ItemType Directory -Path $qa -Force | Out-Null
$qa = (Resolve-Path -LiteralPath $qa).Path
Copy-Item -LiteralPath $Executable -Destination (Join-Path $qa 'mdlite.exe') -Force
$document = Join-Path $qa 'document.md'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\examples\showcase.md') -Destination $document -Force
$config = Join-Path $qa 'mdlite.ini'
# Scroll positions are asserted the instant a key is sent, so this run scrolls
# without the animation. The Rust tests cover the animated path.
function Restore-Config {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\mdlite.ini') -Destination $script:config -Force
    Add-Content -LiteralPath $script:config -Value "`n[viewer]`nsmooth=off"
}
Restore-Config
$timer = [Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath (Join-Path $qa 'mdlite.exe') -ArgumentList ('"' + $document + '"') -WindowStyle Hidden -PassThru
try {
    if (-not $process.WaitForInputIdle(15000)) { throw 'Application did not become ready.' }
    $window = [MDLiteSmoke]::Find($process.Id)
    if ($window -eq [IntPtr]::Zero) { throw 'Native window not found.' }
    $title = [Text.StringBuilder]::new(256)
    do {
        [void][MDLiteSmoke]::GetWindowTextW($window, $title, $title.Capacity)
        if ($title.ToString().StartsWith('document.md')) { break }
        if ($timer.Elapsed.TotalSeconds -gt 15) { throw 'Document did not finish opening.' }
        Start-Sleep -Milliseconds 50
    } while ($true)
    [MDLiteSmoke]::Send($window, 0x8001)
    # DWM may return an empty cached surface for a never-shown window. Map the
    # test window outside the desktop so PrintWindow can request a real paint.
    [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, -32000, -32000, 920, 720, 0x54)
    $startupMs = $timer.Elapsed.TotalMilliseconds
    function Capture([string]$Name, [int]$ExpectedBackground, [bool]$RequireColor = $false) {
        $rect = New-Object MDLiteSmoke+Rect
        [void][MDLiteSmoke]::GetClientRect($window, [ref]$rect)
        $bitmap = New-Object Drawing.Bitmap($rect.Right, $rect.Bottom)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try { if (-not [MDLiteSmoke]::PrintWindow($window, $dc, 1)) { throw 'Window capture failed.' } }
            finally { $graphics.ReleaseHdc($dc) }
            $pixel = $bitmap.GetPixel(4, 4).ToArgb() -band 0xffffff
            $bitmap.Save((Join-Path $qa "$Name.png"))
            if ($pixel -ne $ExpectedBackground) { throw "Wrong background in ${Name}: $pixel" }
            if ($RequireColor) {
                $warmPixels = 0
                for ($y = 75; $y -lt [math]::Min(500, $bitmap.Height); $y += 3) {
                    for ($x = 20; $x -lt [math]::Min(700, $bitmap.Width); $x += 3) {
                        $color = $bitmap.GetPixel($x, $y)
                        if ($color.R -gt 150 -and $color.G -gt 90 -and $color.B -lt 100) { $warmPixels++ }
                    }
                }
                if ($warmPixels -lt 30) { throw "Color emoji pixels missing in $Name." }
            }
        } finally { $graphics.Dispose(); $bitmap.Dispose() }
    }
    # WM_COMMAND 200 + n selects palette n; 104 rereads the INI.
    Capture 'dark' 0x181a1f
    [MDLiteSmoke]::Send($window, 0x111, 201)
    Capture 'light' 0xfaf9f6
    Set-Content -LiteralPath $config -Value "[viewer]`nsmooth=off`n[light]`nbackground=#EFE8DA" -Encoding utf8
    [MDLiteSmoke]::Send($window, 0x111, 104)
    Capture 'custom' 0xefe8da
    $gdiBefore = [MDLiteSmoke]::GetGuiResources($process.Handle, 0)
    for ($i = 0; $i -lt 50; $i++) {
        [MDLiteSmoke]::Send($window, 0x111, (200 + ($i % 2)))
        [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, (500 + ($i % 3) * 150), 500, 0x16)
        [MDLiteSmoke]::Send($window, 0x8001)
    }
    $gdiAfter = [MDLiteSmoke]::GetGuiResources($process.Handle, 0)
    if ($gdiAfter -gt $gdiBefore + 2) { throw "GDI resources leaked: $gdiBefore -> $gdiAfter" }
    $large = "# Large document`n`n" + (("A line with **bold**, ``code``, and Unicode Grüße 世界 😀.`n") * 10000)
    [IO.File]::WriteAllText($document, $large)
    $timer.Restart()
    [MDLiteSmoke]::Send($window, 0x111, 101)
    $reloadMs = $timer.Elapsed.TotalMilliseconds
    $timer.Restart()
    [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 1100, 700, 0x16)
    [MDLiteSmoke]::Send($window, 0x8001)
    $repeatedResizeMs = $timer.Elapsed.TotalMilliseconds
    $unique = [Text.StringBuilder]::new()
    for ($i = 0; $i -lt 10000; $i++) {
        [void]$unique.AppendLine("Row $i has unique text Grüße 世界 😀 and **item $i** with ``value_$i``.")
    }
    [IO.File]::WriteAllText($document, $unique.ToString())
    $timer.Restart()
    [MDLiteSmoke]::Send($window, 0x111, 101)
    $uniqueReloadMs = $timer.Elapsed.TotalMilliseconds
    $timer.Restart()
    [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 1250, 700, 0x16)
    [MDLiteSmoke]::Send($window, 0x8001)
    $uniqueResizeMs = $timer.Elapsed.TotalMilliseconds
    [MDLiteSmoke]::Send($window, 0x100, 0x23)
    $info = New-Object MDLiteSmoke+Scroll
    $info.Size = [Runtime.InteropServices.Marshal]::SizeOf($info)
    $info.Mask = 7
    [void][MDLiteSmoke]::GetScrollInfo($window, 1, [ref]$info)
    if ($info.Pos -le 65535 -or $info.Pos -ne ($info.Max - $info.Page + 1)) { throw 'End / large scrollbar position failed.' }
    [MDLiteSmoke]::Send($window, 0x100, 0x24)
    [void][MDLiteSmoke]::GetScrollInfo($window, 1, [ref]$info)
    if ($info.Pos -ne 0) { throw 'Home key failed.' }
    # F5 must keep a reader's position when reloading the same file.
    [MDLiteSmoke]::Send($window, 0x100, 0x22)
    [void][MDLiteSmoke]::GetScrollInfo($window, 1, [ref]$info)
    $beforeReload = $info.Pos
    [MDLiteSmoke]::Send($window, 0x111, 101)
    [void][MDLiteSmoke]::GetScrollInfo($window, 1, [ref]$info)
    if ($info.Pos -ne $beforeReload) { throw 'Reload lost the reading position.' }
    [MDLiteSmoke]::Send($window, 0x100, 0x24)
    $process.Refresh()
    $largeWorkingSet = $process.WorkingSet64
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot '..\examples\emoji.md') -Destination $document -Force
    Restore-Config
    [MDLiteSmoke]::Send($window, 0x111, 104)
    [MDLiteSmoke]::Send($window, 0x111, 101)
    [MDLiteSmoke]::Send($window, 0x111, 200)
    Capture 'emoji-dark' 0x181a1f $true
    [MDLiteSmoke]::Send($window, 0x111, 201)
    Capture 'emoji-light' 0xfaf9f6 $true
    [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 420, 720, 0x16)
    [MDLiteSmoke]::Send($window, 0x8001)
    Capture 'emoji-narrow' 0xfaf9f6 $true
    $languageGdiBefore = [MDLiteSmoke]::GetGuiResources($process.Handle, 0)
    foreach ($fixture in @('languages', 'complex-scripts')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot "..\examples\$fixture.md") -Destination $document -Force
        [MDLiteSmoke]::Send($window, 0x111, 101)
        foreach ($width in @(920, 360)) {
            [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, $width, 900, 0x16)
            [MDLiteSmoke]::Send($window, 0x8001)
            foreach ($mode in @(200, 201)) {
                [MDLiteSmoke]::Send($window, 0x111, $mode)
                [MDLiteSmoke]::Send($window, 0x100, 0x24)
                for ($page = 0; $page -lt 3; $page++) {
                    $background = if ($mode -eq 200) { 0x181a1f } else { 0xfaf9f6 }
                    Capture "$fixture-$width-$mode-$page" $background
                    [MDLiteSmoke]::Send($window, 0x100, 0x22)
                }
            }
        }
    }
    # Repeated complex paragraphs share text and native layouts too.
    [IO.File]::WriteAllText($document, ("العَرَبِيَّةُ **نص عربي** English 123 😀`n" * 10000))
    $timer.Restart()
    [MDLiteSmoke]::Send($window, 0x111, 101)
    $complexReloadMs = $timer.Elapsed.TotalMilliseconds
    $timer.Restart()
    [void][MDLiteSmoke]::SetWindowPos($window, [IntPtr]::Zero, 0, 0, 1100, 700, 0x16)
    [MDLiteSmoke]::Send($window, 0x8001)
    $complexResizeMs = $timer.Elapsed.TotalMilliseconds
    [MDLiteSmoke]::Send($window, 0x100, 0x24)
    Capture 'complex-repeated' 0xfaf9f6
    # Exercise cache eviction, theme switching, resizing and actual painting.
    $uniqueComplex = [Text.StringBuilder]::new()
    for ($i = 0; $i -lt 200; $i++) { [void]$uniqueComplex.AppendLine("العربية **نص $i** हिन्दी café 😀 [رابط](https://example.test)") }
    [IO.File]::WriteAllText($document, $uniqueComplex.ToString())
    for ($i = 0; $i -lt 8; $i++) {
        [MDLiteSmoke]::Send($window, 0x111, 101)
        [MDLiteSmoke]::Send($window, 0x111, (200 + ($i % 2)))
        [MDLiteSmoke]::Send($window, 0x100, 0x23)
        Capture 'complex-eviction' $(if ($i % 2) { 0xfaf9f6 } else { 0x181a1f })
    }
    $languageGdiAfter = [MDLiteSmoke]::GetGuiResources($process.Handle, 0)
    if ($languageGdiAfter -gt $languageGdiBefore + 2) { throw "Multilingual GDI resources leaked: $languageGdiBefore -> $languageGdiAfter" }
    $process.Refresh()
    $cpuBefore = $process.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Milliseconds 1000
    $process.Refresh()
    $idleCpuMs = $process.TotalProcessorTime.TotalMilliseconds - $cpuBefore
    $result = [pscustomobject]@{
        Result = 'PASS'
        StartupMilliseconds = [math]::Round($startupMs, 1)
        Reload10000LinesMilliseconds = [math]::Round($reloadMs, 1)
        ResizeRepeatedLinesMilliseconds = [math]::Round($repeatedResizeMs, 1)
        Reload10000UniqueLinesMilliseconds = [math]::Round($uniqueReloadMs, 1)
        ResizeUniqueLinesMilliseconds = [math]::Round($uniqueResizeMs, 1)
        Reload10000ComplexLinesMilliseconds = [math]::Round($complexReloadMs, 1)
        ResizeComplexLinesMilliseconds = [math]::Round($complexResizeMs, 1)
        LanguageGdiObjectsBefore = $languageGdiBefore
        LanguageGdiObjectsAfter = $languageGdiAfter
        WorkingSetMiB = [math]::Round($largeWorkingSet / 1MB, 1)
        IdleCpuMillisecondsOverOneSecond = $idleCpuMs
        GdiObjectsBefore = $gdiBefore
        GdiObjectsAfter = $gdiAfter
        Screenshots = $qa
    }
    $result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $qa $Report) -Encoding utf8
    $result | Format-List
} finally {
    if (-not $process.HasExited) {
        $window = [MDLiteSmoke]::Find($process.Id)
        if ($window -ne [IntPtr]::Zero) { [MDLiteSmoke]::Send($window, 0x10) }
        if (-not $process.WaitForExit(5000)) { $process.Kill() }
    }
    $process.Dispose()
}
