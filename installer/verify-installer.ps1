# Installs MDLite from the built installer into a scratch folder, checks what it
# did, uninstalls it, and checks that nothing was left behind. Per-user, so no
# administrator rights; it does touch this account's registry, which is the point,
# and it compares that registry with a snapshot taken first.
#
#   .\installer\verify-installer.ps1 [-Setup path\to\MDLite-Setup-x.y.z.exe] [-RunningApp] [-Upgrade]
#
# -RunningApp leaves MDLite open while uninstalling, to show it is closed for you.
# -Upgrade installs a second time over the first while MDLite is open.
param(
    [string]$Setup = (Get-ChildItem (Join-Path $PSScriptRoot '..\dist\installer\MDLite-Setup-*.exe') | Sort-Object LastWriteTime | Select-Object -Last 1).FullName,
    [switch]$RunningApp,
    [switch]$Upgrade
)
$ErrorActionPreference = 'Stop'
if (-not $Setup -or -not (Test-Path -LiteralPath $Setup)) { throw 'Build the installer first: .\installer\build-installer.ps1' }
$work = Join-Path $env:TEMP "mdlite-verify-$PID"
$dir = Join-Path $work 'app'
$fakeAppData = Join-Path $work 'appdata'
New-Item -ItemType Directory -Path $work, $fakeAppData -Force | Out-Null
$failures = 0
function Check([string]$What, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) { "  ok    $What" } else { "  FAIL  $What  $Detail"; $script:failures++ }
}
# Everything MDLite may write for this user: keys, values and data, as text.
function Snapshot {
    $lines = [Collections.Generic.List[string]]::new()
    $keys = 'Software\Classes\MDLite.Markdown', 'Software\Classes\Applications\mdlite.exe', 'Software\MDLite',
        'Software\Classes\.md', 'Software\Classes\.markdown', 'Software\Classes\.mdown', 'Software\Classes\.mkd'
    foreach ($key in $keys) {
        $path = "HKCU:\$key"
        if (-not (Test-Path -LiteralPath $path)) { continue }
        # A key with nothing in it is the same as no key, so only values are compared.
        foreach ($item in @(Get-Item -LiteralPath $path) + @(Get-ChildItem -LiteralPath $path -Recurse -ErrorAction SilentlyContinue)) {
            foreach ($name in $item.GetValueNames()) { $lines.Add("$($item.Name) [$name] = $($item.GetValue($name))") }
        }
    }
    $registered = (Get-ItemProperty 'HKCU:\Software\RegisteredApplications' -ErrorAction SilentlyContinue).MDLite
    $lines.Add("RegisteredApplications MDLite = $registered")
    $lines
}
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{6893FA32-7C9A-4FA6-9700-929922862FFE}_is1'
function Wait-Gone([string]$Path, [int]$Seconds = 90) {
    $until = (Get-Date).AddSeconds($Seconds)
    while ((Test-Path -LiteralPath $Path) -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 300 }
    -not (Test-Path -LiteralPath $Path)
}
try {
    $before = Snapshot
    $claimedBefore = (Get-ItemProperty 'Registry::HKEY_CLASSES_ROOT\.md' -ErrorAction SilentlyContinue).'(default)'
    "Installer: $Setup"
    "`.md opens with before: $(if ($claimedBefore) { $claimedBefore } else { '(nothing)' })"

    "`nInstall"
    $log = Join-Path $work 'setup.log'
    $run = Start-Process -FilePath $Setup -Wait -PassThru -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$dir`"", '/TASKS=assoc', "/LOG=`"$log`""
    Check 'setup exited 0' ($run.ExitCode -eq 0) "exit $($run.ExitCode), see $log"
    foreach ($file in 'mdlite.exe', 'mdlite.default.ini', 'README.md', 'LICENSE.txt', 'examples\showcase.md', 'unins000.exe') {
        Check "installed $file" (Test-Path -LiteralPath (Join-Path $dir $file))
    }
    Check 'no live mdlite.ini beside the program' (-not (Test-Path -LiteralPath (Join-Path $dir 'mdlite.ini')))
    $command = (Get-ItemProperty 'HKCU:\Software\Classes\MDLite.Markdown\shell\open\command').'(default)'
    Check 'open command points at the install' ($command -eq "`"$dir\mdlite.exe`" `"%1`"") $command
    Check 'listed under Open with for .md' ($null -ne (Get-Item 'HKCU:\Software\Classes\.md\OpenWithProgids').GetValue('MDLite.Markdown'))
    Check 'listed in Default apps capabilities' ((Get-ItemProperty 'HKCU:\Software\MDLite\Capabilities\FileAssociations').'.mkd' -eq 'MDLite.Markdown')
    Check 'registered application' ((Get-ItemProperty 'HKCU:\Software\RegisteredApplications').MDLite -eq 'Software\MDLite\Capabilities')
    if (-not $claimedBefore) {
        Check '.md is MDLite when nothing else had it' ((Get-ItemProperty 'HKCU:\Software\Classes\.md').'(default)' -eq 'MDLite.Markdown')
    } else {
        Check ".md left with $claimedBefore" ((Get-ItemProperty 'Registry::HKEY_CLASSES_ROOT\.md').'(default)' -eq $claimedBefore)
    }
    Check 'listed in Apps & features' (Test-Path -LiteralPath $uninstallKey)
    Check 'Start menu shortcut' (Test-Path -LiteralPath (Join-Path ([Environment]::GetFolderPath('Programs')) 'MDLite.lnk'))

    "`nFirst run, with its own empty settings folder"
    $savedAppData = $env:APPDATA
    $env:APPDATA = $fakeAppData
    $app = Start-Process -FilePath (Join-Path $dir 'mdlite.exe') -ArgumentList ('"' + (Join-Path $dir 'examples\showcase.md') + '"') -PassThru
    $env:APPDATA = $savedAppData
    $ini = Join-Path $fakeAppData 'MDLite\mdlite.ini'
    $until = (Get-Date).AddSeconds(15)
    while (-not (Test-Path -LiteralPath $ini) -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 200 }
    Check 'settings file created in the profile' (Test-Path -LiteralPath $ini)
    Check 'started from the shipped palettes' ((Test-Path -LiteralPath $ini) -and ((Get-Content -Raw -LiteralPath $ini) -match '\[Solarized Dark\]'))
    if ($Upgrade) {
        "`nUpgrade over the open program"
        $run = Start-Process -FilePath $Setup -Wait -PassThru -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=`"$dir`"", '/TASKS=assoc'
        Check 'second setup exited 0' ($run.ExitCode -eq 0) "exit $($run.ExitCode)"
        Check 'the open program was closed for it' ($app.WaitForExit(10000))
        $entries = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' | Where-Object { $_.PSChildName -like '{6893FA32*' })
        Check 'one uninstall entry, not two' ($entries.Count -eq 1) "$($entries.Count) entries"
    }
    if (-not $RunningApp) { if (-not $app.HasExited) { $app.Kill() }; $app.WaitForExit(5000) | Out-Null }

    "`nUninstall$(if ($RunningApp) { ' (MDLite still open)' })"
    $settings = Join-Path $env:APPDATA 'MDLite'
    $real = Test-Path -LiteralPath $settings
    # Seed a file in the real settings folder only if it is not there, so
    # "kept" and "purged" can be told apart without losing anyone's settings.
    $marker = $null
    if (-not $real) {
        New-Item -ItemType Directory -Path $settings -Force | Out-Null
        $marker = Join-Path $settings 'verify.marker'
        Set-Content -LiteralPath $marker -Value 'x'
    }
    $run = Start-Process -FilePath (Join-Path $dir 'unins000.exe') -Wait -PassThru -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
    Check 'uninstaller exited 0' ($run.ExitCode -eq 0) "exit $($run.ExitCode)"
    Check 'program folder removed' (Wait-Gone $dir) "$dir is still there"
    Check 'Apps & features entry removed' (-not (Test-Path -LiteralPath $uninstallKey))
    Check 'Start menu shortcut removed' (-not (Test-Path -LiteralPath (Join-Path ([Environment]::GetFolderPath('Programs')) 'MDLite.lnk')))
    if ($marker) { Check 'a silent uninstall keeps your settings' (Test-Path -LiteralPath $marker); Remove-Item -LiteralPath $settings -Recurse -Force }
    $after = Snapshot
    $difference = Compare-Object $before $after
    Check 'registry is exactly as it was before' (-not $difference) (($difference | ForEach-Object { "$($_.SideIndicator) $($_.InputObject)" }) -join '; ')
    if ($app -and -not $app.HasExited) { $app.Kill() }
}
finally {
    $env:APPDATA = if ($savedAppData) { $savedAppData } else { $env:APPDATA }
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
"`n$(if ($failures) { "$failures check(s) FAILED" } else { 'All checks passed' })"
exit $(if ($failures) { 1 } else { 0 })
