#Requires -Version 5.1
<#
.SYNOPSIS
  DeskCraft launcher: desktop egui app (L3), CLI/MCP, or smoke check.
#>
param(
    [switch]$Mcp,
    [switch]$Smoke,
    [switch]$Build
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$AppCrate = "deskcraft-app"
$AppBin = "deskcraft-app"
$McpCrate = "deskcraft-mcp"

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
if ($null -eq (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: cargo not found. Install Rust: https://rustup.rs" -ForegroundColor Red
    exit 1
}

Set-Location -LiteralPath $RepoRoot

if ($Mcp) {
    Write-Host "Starting DeskCraft MCP server on stdio..." -ForegroundColor Cyan
    & "$env:USERPROFILE\.cargo\bin\cargo.exe" run -p $McpCrate -- mcp
    exit $LASTEXITCODE
}

if ($Smoke -or ($args -contains "--smoke")) {
    & "$env:USERPROFILE\.cargo\bin\cargo.exe" run -p $AppCrate -- --smoke
    exit $LASTEXITCODE
}

if ($Build) {
    & "$env:USERPROFILE\.cargo\bin\cargo.exe" build -p $AppCrate
    exit $LASTEXITCODE
}

# Default: Launch native desktop app
$releaseExe = Join-Path $RepoRoot "target\release\$AppBin.exe"
$debugExe = Join-Path $RepoRoot "target\debug\$AppBin.exe"

if (Test-Path $releaseExe) {
    & $releaseExe @args
    exit $LASTEXITCODE
} elseif (Test-Path $debugExe) {
    & $debugExe @args
    exit $LASTEXITCODE
} else {
    & "$env:USERPROFILE\.cargo\bin\cargo.exe" run -p $AppCrate -- @args
    exit $LASTEXITCODE
}
