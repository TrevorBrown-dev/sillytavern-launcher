# Builds the frontend (Vite) and the release exe (cargo), then copies the exe to
# release\SillyTavern Launcher.exe.
# Note: cargo and npm print progress on stderr, so don't use $ErrorActionPreference = 'Stop'
# around them in Windows PowerShell 5.1 (that turns the first stderr line into a fatal error).
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot

Push-Location $root
try {
    if (-not (Test-Path "$root\node_modules")) {
        npm ci --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed with exit code $LASTEXITCODE" }
    }
    npm run build
    if ($LASTEXITCODE -ne 0) { throw "vite build failed with exit code $LASTEXITCODE" }

    Push-Location "$root\src-tauri"
    try {
        # custom-protocol = production mode (embedded dist/ rather than the Vite dev URL).
        cargo build --release --features custom-protocol
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
} finally {
    Pop-Location
}

New-Item -ItemType Directory -Force "$root\release" | Out-Null
Copy-Item "$root\src-tauri\target\release\stlauncher.exe" "$root\release\SillyTavern Launcher.exe" -Force
Get-Item "$root\release\SillyTavern Launcher.exe" | Select-Object FullName, Length, LastWriteTime
