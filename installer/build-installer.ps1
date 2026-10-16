# Builds MDLite and wraps it in an installer: dist\installer\MDLite-Setup-<version>.exe.
#
#   .\installer\build-installer.ps1            build the program, then the installer
#   .\installer\build-installer.ps1 -Test      ... running the tests first
#   .\installer\build-installer.ps1 -SkipBuild reuse dist\mdlite.exe as it is
#
# Needs Inno Setup 6.4 or later:  winget install JRSoftware.InnoSetup
param(
    [switch]$Test,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# The version lives in one place: Cargo.toml.
$manifest = Get-Content -Raw -LiteralPath (Join-Path $root 'Cargo.toml')
if ($manifest -notmatch '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"') { throw 'No version found in Cargo.toml.' }
$version = $Matches[1]

if (-not $SkipBuild) {
    if ($Test) { & (Join-Path $root 'build.ps1') -Test | Out-Null } else { & (Join-Path $root 'build.ps1') | Out-Null }
}
$exe = Join-Path $root 'dist\mdlite.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "Missing $exe. Run .\build.ps1 first." }

$candidates = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
if (-not $candidates) { throw 'Inno Setup 6 was not found. Install it with: winget install JRSoftware.InnoSetup' }
$iscc = @($candidates)[0]

$output = Join-Path $root 'dist\installer'
New-Item -ItemType Directory -Path $output -Force | Out-Null
& $iscc /Qp "/DAppVersion=$version" "/DSourceDir=$(Join-Path $root 'dist')" "/DOutputDir=$output" (Join-Path $PSScriptRoot 'mdlite.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed with exit code $LASTEXITCODE." }

$setup = Join-Path $output "MDLite-Setup-$version.exe"
[pscustomobject]@{
    Installer = $setup
    KiB = [math]::Round((Get-Item -LiteralPath $setup).Length / 1KB, 1)
    Version = $version
    SHA256 = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash
} | Format-List
