#!/usr/bin/env pwsh
# Lanceur PowerShell multi-plateforme (Windows x86_64 / macOS / Linux)
$ErrorActionPreference = "Stop"

$RootDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$CargoManifest = Join-Path $RootDir "tools/blueorb/Cargo.toml"

if ($IsWindows -or $env:OS -eq "Windows_NT") {
    $CargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    $env:PATH = "$CargoBin;C:\devkitPro\devkitARM\bin;C:\Program Files\LLVM\bin;$env:PATH"
    $Bin = Join-Path $RootDir "tools\blueorb\target\release\blueorb.exe"
} else {
    $env:PATH = "/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:/opt/devkitpro/devkitARM/bin:$env:PATH"
    $Bin = Join-Path $RootDir "tools/blueorb/target/release/blueorb"
}

if (Get-Command cargo -ErrorAction SilentlyContinue) {
    & cargo build --release --manifest-path $CargoManifest --quiet
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} elseif (-not (Test-Path $Bin)) {
    Write-Error "Rust (cargo) est introuvable. Sur Windows x86_64: winget install -e --id Rustlang.Rustup"
    exit 1
}

& $Bin @args
exit $LASTEXITCODE
