# Builds the release exe and copies it to dist\SillyTavern Launcher.exe
# Note: cargo prints progress on stderr, so don't use $ErrorActionPreference = 'Stop'
# around it in Windows PowerShell 5.1 (that turns the first stderr line into a fatal error).
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot

Push-Location "$root\src-tauri"
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force "$root\dist" | Out-Null
Copy-Item "$root\src-tauri\target\release\stlauncher.exe" "$root\dist\SillyTavern Launcher.exe" -Force
Get-Item "$root\dist\SillyTavern Launcher.exe" | Select-Object FullName, Length, LastWriteTime
