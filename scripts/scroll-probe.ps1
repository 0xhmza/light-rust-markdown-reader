# Launches a copy of the viewer off-screen, spins the wheel, and samples the scroll
# position to show how the animation really runs: how often the view moves, how
# evenly, and that the process goes quiet again afterwards.
param(
    [string]$Executable = (Join-Path $PSScriptRoot '..\dist\mdlite.exe'),
    [int]$Notches = 5,
    [switch]$Smooth = $true
)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class MDLiteProbe {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [StructLayout(LayoutKind.Sequential)] public struct Scroll { public uint Size, Mask; public int Min, Max; public uint Page; public int Pos, Track; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int count);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool GetScrollInfo(IntPtr hwnd, int bar, ref Scroll info);
    public static IntPtr Find(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, param) => { uint pid; GetWindowThreadProcessId(hwnd, out pid); if (pid == processId) { var name = new StringBuilder(256); GetClassNameW(hwnd, name, name.Capacity); if (name.ToString() == "MDLite.NativeReader") { found = hwnd; return false; } } return true; }, IntPtr.Zero);
        return found;
    }
    public static int Position(IntPtr hwnd) { var info = new Scroll(); info.Size = (uint)Marshal.SizeOf(info); info.Mask = 4; GetScrollInfo(hwnd, 1, ref info); return info.Pos; }
}
'@
$qa = Join-Path $env:TEMP ('mdlite-probe-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $qa -Force | Out-Null
Copy-Item -LiteralPath $Executable -Destination (Join-Path $qa 'mdlite.exe')
$showcase = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\examples\showcase.md')
Set-Content -LiteralPath (Join-Path $qa 'document.md') -Value ($showcase * 40) -Encoding utf8
Set-Content -LiteralPath (Join-Path $qa 'mdlite.ini') -Value ("[viewer]`nsmooth=" + $(if ($Smooth) { 'on' } else { 'off' })) -Encoding utf8
$process = Start-Process -FilePath (Join-Path $qa 'mdlite.exe') -ArgumentList ('"' + (Join-Path $qa 'document.md') + '"') -WindowStyle Hidden -PassThru
try {
    if (-not $process.WaitForInputIdle(15000)) { throw 'Not ready.' }
    $window = [MDLiteProbe]::Find($process.Id)
    if ($window -eq [IntPtr]::Zero) { throw 'No window.' }
    Start-Sleep -Milliseconds 800
    # Mapped far off the desktop, so nothing flashes on the reader's screen.
    [void][MDLiteProbe]::SetWindowPos($window, [IntPtr]::Zero, -32000, -32000, 1000, 800, 0x54)
    Start-Sleep -Milliseconds 800
    $start = [MDLiteProbe]::Position($window)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    # WM_MOUSEWHEEL, one notch each, a few milliseconds apart like a real wheel.
    for ($i = 0; $i -lt $Notches; $i++) {
        [void][MDLiteProbe]::PostMessageW($window, 0x20A, [IntPtr](-120 -shl 16), [IntPtr]0)
        Start-Sleep -Milliseconds 12
    }
    $samples = New-Object Collections.Generic.List[object]
    $last = $start
    while ($watch.ElapsedMilliseconds -lt 1200) {
        $now = [MDLiteProbe]::Position($window)
        if ($now -ne $last) { $samples.Add([pscustomobject]@{ Ms = $watch.Elapsed.TotalMilliseconds; Pos = $now; Step = $now - $last }); $last = $now }
        Start-Sleep -Milliseconds 1
    }
    $gaps = for ($i = 1; $i -lt $samples.Count; $i++) { $samples[$i].Ms - $samples[$i - 1].Ms }
    $busy = $gaps | Where-Object { $_ -lt 60 }
    $process.Refresh()
    $cpu0 = $process.TotalProcessorTime.TotalMilliseconds
    Start-Sleep -Milliseconds 1000
    $process.Refresh()
    $idle = $process.TotalProcessorTime.TotalMilliseconds - $cpu0
    [pscustomobject]@{
        Smooth = [bool]$Smooth
        Notches = $Notches
        DistancePx = $last - $start
        PositionChanges = $samples.Count
        SettledAfterMs = if ($samples.Count) { [math]::Round($samples[$samples.Count - 1].Ms) } else { 0 }
        MeanFrameMs = if ($busy) { [math]::Round(($busy | Measure-Object -Average).Average, 1) } else { 0 }
        WorstGapMs = if ($busy) { [math]::Round(($busy | Measure-Object -Maximum).Maximum, 1) } else { 0 }
        BiggestStepPx = if ($samples.Count) { ($samples | Measure-Object -Property Step -Maximum).Maximum } else { 0 }
        IdleCpuMsOverOneSecond = [math]::Round($idle, 1)
    } | Format-List
    if ($samples.Count) { "first steps: " + (($samples | Select-Object -First 14 | ForEach-Object { $_.Step }) -join ' ') }
} finally {
    if (-not $process.HasExited) { $process.Kill() }
    [void]$process.WaitForExit(5000)
    $process.Dispose()
    Remove-Item -LiteralPath $qa -Recurse -Force -ErrorAction SilentlyContinue
}
