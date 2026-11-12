#Requires -Version 5.1
<#
.SYNOPSIS
    Start a craft app. VENDORED from mcp-central-docs\templates\craft-app (identical in every
    craft repo); per-repo settings are in scripts\craft.config.psd1.

.DESCRIPTION
    Default: incremental cargo build of the desktop app, then run it, so the window always
    runs the current code (an up-to-date build takes a second or two).
    If the app is already running and the code is newer than that running instance, it says
    so and stops instead of starting a second, different window.

.PARAMETER Release
    Optimized build (target\release).
.PARAMETER NoBuild
    Run the last build as it is.
.PARAMETER New
    Start another window even if one is running.
.PARAMETER Smoke
    Headless check on a throwaway database (no window): app smoke and CLI smoke from the config.
.PARAMETER Mcp
    Run the MCP stdio server (the CLI binary).
.PARAMETER Cli
    Run the CLI with the remaining arguments.

.EXAMPLE
    .\start.ps1
    .\start.ps1 -Release --dark --lang en
    .\start.ps1 -Smoke
    .\start.ps1 -Cli status
#>
param(
    [switch]$Release,
    [switch]$NoBuild,
    [switch]$New,
    [switch]$Smoke,
    [switch]$Mcp,
    [switch]$Cli,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Rest
)

$ErrorActionPreference = 'Continue'
$RepoRoot = $PSScriptRoot
. (Join-Path $RepoRoot 'scripts\craft-common.ps1')
Initialize-CraftCargo
$cfg = Get-CraftConfig $RepoRoot
Set-Location -LiteralPath $RepoRoot
if ($null -eq $Rest) { $Rest = @() }
$mode = if ($Release) { 'release' } else { 'debug' }
$targetDir = Get-CraftTargetDir $RepoRoot
function Exe([string]$bin) { Join-Path (Join-Path $targetDir $mode) "$bin.exe" }

function Build([string]$pkg, [string]$bin) {
    if ($NoBuild) { return }
    $exe = Exe $bin
    if (Get-Process -Name $bin -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
        Write-Host "[$($cfg.Name)] $bin is running from $exe, so it cannot be rebuilt now: using that build" -ForegroundColor Yellow
        return
    }
    Write-Host "[$($cfg.Name)] building $bin ($mode)..." -ForegroundColor Cyan
    $code = Invoke-CraftBuild -Package $pkg -Bin $bin -Release:$Release
    if ($code -ne 0) { Write-Host "[$($cfg.Name)] build failed (exit $code)" -ForegroundColor Red; exit $code }
}

function Run([string]$bin, [object[]]$argList) {
    $exe = Exe $bin
    if (-not (Test-Path -LiteralPath $exe)) { Write-Host "[$($cfg.Name)] not built: $exe (drop -NoBuild)" -ForegroundColor Red; exit 1 }
    & $exe @argList
    exit $LASTEXITCODE
}

if ($Mcp) {
    if (-not $cfg.McpBin) { Write-Host "[$($cfg.Name)] this app has no MCP binary (scripts\craft.config.psd1)" -ForegroundColor Red; exit 1 }
    Build $cfg.McpPackage $cfg.McpBin
    Write-Host "[$($cfg.Name)] MCP stdio server; Ctrl+C to stop" -ForegroundColor Cyan
    Run $cfg.McpBin (@($cfg.McpArgs) + $Rest)
}
if ($Cli) {
    if (-not $cfg.CliBin) { Write-Host "[$($cfg.Name)] this app has no CLI binary (scripts\craft.config.psd1)" -ForegroundColor Red; exit 1 }
    Build $cfg.CliPackage $cfg.CliBin
    Run $cfg.CliBin $Rest
}

if ($Smoke) {
    $db = Join-Path ([IO.Path]::GetTempPath()) "$($cfg.Name)-start-smoke-$PID.db"
    $failed = 0
    if ($cfg.AppBin -and @($cfg.AppSmoke).Count -gt 0) {
        Build $cfg.AppPackage $cfg.AppBin
        & (Exe $cfg.AppBin) @(Expand-CraftArgs $cfg.AppSmoke $db)
        if ($LASTEXITCODE -ne 0) { $failed = $LASTEXITCODE; Write-Host "[$($cfg.Name)] app smoke failed" -ForegroundColor Red }
    }
    if ($cfg.CliBin -and @($cfg.CliSmoke).Count -gt 0) {
        Build $cfg.CliPackage $cfg.CliBin
        & (Exe $cfg.CliBin) @(Expand-CraftArgs $cfg.CliSmoke $db)
        if ($LASTEXITCODE -ne 0) { $failed = $LASTEXITCODE; Write-Host "[$($cfg.Name)] cli smoke failed" -ForegroundColor Red }
    }
    Remove-Item "$db*" -Force -ErrorAction SilentlyContinue
    if (@($cfg.AppSmoke).Count -eq 0 -and @($cfg.CliSmoke).Count -eq 0) {
        if ($cfg.AppBin) { Build $cfg.AppPackage $cfg.AppBin }
        if ($cfg.CliBin) { Build $cfg.CliPackage $cfg.CliBin }
        Write-Host "[$($cfg.Name)] built; no smoke configured (AppSmoke / CliSmoke in scripts\craft.config.psd1)" -ForegroundColor Yellow
        exit 0
    }
    if ($failed -eq 0) { Write-Host "[$($cfg.Name)] smoke ok" -ForegroundColor Green }
    exit $failed
}

if (-not $cfg.AppBin) {
    Write-Host "[$($cfg.Name)] no desktop app configured; running the CLI" -ForegroundColor Yellow
    Build $cfg.CliPackage $cfg.CliBin
    Run $cfg.CliBin $Rest
}

$running = Get-Process -Name $cfg.AppBin -ErrorAction SilentlyContinue
if ($running -and -not $New) {
    $started = ($running | Sort-Object StartTime | Select-Object -First 1).StartTime
    $newest = Get-CraftNewestSource $RepoRoot
    if ($newest -gt $started) {
        Write-Host "[$($cfg.Name)] a window started $started is still open, but the code changed at $newest." -ForegroundColor Yellow
        Write-Host "[$($cfg.Name)] close it and run start again for the current build (or pass -New for a second window)." -ForegroundColor Yellow
        exit 2
    }
    Write-Host "[$($cfg.Name)] already running (PID $($running[0].Id)), up to date. Pass -New for a second window." -ForegroundColor Green
    exit 0
}

Build $cfg.AppPackage $cfg.AppBin
Write-Host "[$($cfg.Name)] starting $(Exe $cfg.AppBin)" -ForegroundColor Cyan
Run $cfg.AppBin (@($cfg.AppArgs) + $Rest)
