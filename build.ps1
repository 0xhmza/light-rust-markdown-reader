param(
    # 'small' is the shipped build: same measured speed, a smaller EXE.
    [ValidateSet('small', 'release')][string]$Profile = 'small',
    [switch]$Test
)
$ErrorActionPreference = 'Stop'
Push-Location $PSScriptRoot
$savedRustup = $env:RUSTUP_HOME
$savedCargo = $env:CARGO_HOME
$savedPath = $env:PATH
try {
    $localCargo = Join-Path $PSScriptRoot '.tools\cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $localCargo) {
        $env:RUSTUP_HOME = Join-Path $PSScriptRoot '.tools\rustup'
        $env:CARGO_HOME = Join-Path $PSScriptRoot '.tools\cargo'
        $env:PATH = (Join-Path $env:CARGO_HOME 'bin') + ';' + $env:PATH
    }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Install stable Rust and its Windows linker first: https://www.rust-lang.org/tools/install'
    }
    if ($Test) {
        & cargo test --offline
        if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
    }
    & cargo build --offline --profile $Profile
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    Copy-Item -LiteralPath "target\$Profile\mdlite.exe" -Destination dist\mdlite.exe
    # The toolchain has no resource compiler, so the icon goes on afterwards.
    if (-not (Test-Path -LiteralPath assets\mdlite.ico)) { & scripts\make-icon.ps1 | Out-Null }
    & scripts\set-icon.ps1 | Out-Null
    # Preserve any palette the user has already customized.
    if (-not (Test-Path -LiteralPath dist\mdlite.ini)) {
        Copy-Item -LiteralPath mdlite.ini -Destination dist\mdlite.ini
    }
    Get-Item -LiteralPath dist\mdlite.exe | Select-Object FullName, Length
} finally {
    $env:RUSTUP_HOME = $savedRustup
    $env:CARGO_HOME = $savedCargo
    $env:PATH = $savedPath
    Pop-Location
}
