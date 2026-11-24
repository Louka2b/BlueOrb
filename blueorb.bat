@echo off
REM Lanceur natif Windows x86_64 (Invite de commandes cmd.exe / PowerShell)
setlocal

set "ROOT_DIR=%~dp0"
if "%ROOT_DIR:~-1%"=="\" set "ROOT_DIR=%ROOT_DIR:~0,-1%"
set "PATH=%USERPROFILE%\.cargo\bin;C:\devkitPro\devkitARM\bin;C:\Program Files\LLVM\bin;%PATH%"

set "BIN=%ROOT_DIR%\tools\blueorb\target\release\blueorb.exe"

where cargo >nul 2>nul
if errorlevel 1 (
    if not exist "%BIN%" (
        echo [erreur] Rust ^(cargo.exe^) est introuvable. Installez Rust sur Windows x86_64 avec :
        echo   winget install -e --id Rustlang.Rustup
        exit /b 1
    )
) else (
    cargo build --release --manifest-path "%ROOT_DIR%\tools\blueorb\Cargo.toml" --quiet
    if errorlevel 1 exit /b 1
)

"%BIN%" %*
exit /b %ERRORLEVEL%
