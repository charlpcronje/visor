#!/usr/bin/env pwsh
# Build visor and copy to C:\dev\scripts\visor.exe

$ErrorActionPreference = "Stop"

Write-Host "Building visor (release)..." -ForegroundColor Cyan
cargo build --release

if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!" -ForegroundColor Red
    exit 1
}

$src = "target\release\visor.exe"
$dst = "C:\dev\scripts\visor.exe"

# Ensure destination directory exists
if (-not (Test-Path "C:\dev\scripts")) {
    New-Item -ItemType Directory -Path "C:\dev\scripts" -Force | Out-Null
}

Copy-Item $src $dst -Force
Write-Host "Copied to $dst" -ForegroundColor Green
Write-Host "Done." -ForegroundColor Green
