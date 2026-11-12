# craft-common.ps1 - shared helpers for start.ps1 and scripts\release.ps1 of a Rust craft app.
# VENDORED from mcp-central-docs\templates\craft-app. Do not edit in an app repo: change the
# template in mcd and run mcp-central-docs\scripts\sync-craft-scaffold.ps1. Per-repo settings
# live in scripts\craft.config.psd1. Windows PowerShell 5.1 compatible, ASCII only.

function Initialize-CraftCargo {
    $bin = Join-Path $env:USERPROFILE '.cargo\bin'
    if ((Test-Path -LiteralPath $bin) -and ($env:Path -notlike "*$bin*")) { $env:Path = "$bin;$env:Path" }
    if ($null -eq (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Host '[ERROR] cargo not found. Install Rust from https://rustup.rs' -ForegroundColor Red
        exit 1
    }
}

function Get-CraftConfig {
    param([string]$RepoRoot)
    $path = Join-Path $RepoRoot 'scripts\craft.config.psd1'
    if (-not (Test-Path -LiteralPath $path)) {
        Write-Host "[ERROR] $path is missing. Run mcp-central-docs\scripts\sync-craft-scaffold.ps1 -Repo <name>." -ForegroundColor Red
        exit 1
    }
    $c = Import-PowerShellDataFile -LiteralPath $path
    $defaults = @{
        Display = $c.Name; AppPackage = $null; AppBin = $null; AppArgs = @()
        CliPackage = $null; CliBin = $null; McpPackage = $null; McpBin = $null
        AppSmoke = @(); CliSmoke = @(); McpArgs = @(); McpSmoke = @()
        Installer = $true; Mcpb = $false; LauncherZip = $true
        Repository = "https://github.com/sandraschi/$($c.Name)"
    }
    foreach ($k in $defaults.Keys) { if (-not $c.ContainsKey($k)) { $c[$k] = $defaults[$k] } }
    # The MCP server is the CLI unless the repo ships a separate one (moneycraft-mcp, aicraft-mcp).
    if (-not $c.McpBin) { $c.McpBin = $c.CliBin; $c.McpPackage = $c.CliPackage }
    return $c
}

# Real target directory (respects CARGO_TARGET_DIR and .cargo/config.toml).
function Get-CraftTargetDir {
    param([string]$RepoRoot)
    $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try {
        Push-Location -LiteralPath $RepoRoot
        $json = & cargo metadata --format-version 1 --no-deps 2>$null | Out-String
        if ($LASTEXITCODE -eq 0 -and $json) { return ($json | ConvertFrom-Json).target_directory }
    } catch {
    } finally {
        Pop-Location
        $ErrorActionPreference = $prev
    }
    if ($env:CARGO_TARGET_DIR) { return $env:CARGO_TARGET_DIR }
    return (Join-Path $RepoRoot 'target')
}

# Version of a workspace package (falls back to [workspace.package] version).
function Get-CraftVersion {
    param([string]$RepoRoot, [string]$Package)
    $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try {
        Push-Location -LiteralPath $RepoRoot
        $json = & cargo metadata --format-version 1 --no-deps 2>$null | Out-String
        if ($LASTEXITCODE -eq 0 -and $json) {
            $p = ($json | ConvertFrom-Json).packages | Where-Object { $_.name -eq $Package } | Select-Object -First 1
            if ($p) { return $p.version }
        }
    } catch {
    } finally {
        Pop-Location
        $ErrorActionPreference = $prev
    }
    return $null
}

# Replace {db} in an argument list with a throwaway database path.
function Expand-CraftArgs {
    param([object[]]$ArgList, [string]$Db)
    $out = @()
    foreach ($a in $ArgList) { $out += ([string]$a).Replace('{db}', $Db) }
    return $out   # unrolled; callers wrap in @()
}

function Invoke-CraftBuild {
    param([string]$Package, [string]$Bin, [switch]$Release)
    $flags = @('build')
    if ($Release) { $flags += '--release' }
    $flags += @('-p', $Package, '--bin', $Bin)
    $prev = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    & cargo @flags | Out-Host
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prev
    return $code
}

# Newest *.rs / Cargo.toml under the repo, ignoring build output and VCS folders.
function Get-CraftNewestSource {
    param([string]$RepoRoot)
    $skip = @('target', '.git', 'node_modules', 'dist', '_junk')
    $newest = [datetime]::MinValue
    foreach ($d in Get-ChildItem -LiteralPath $RepoRoot -Directory -ErrorAction SilentlyContinue | Where-Object { $skip -notcontains $_.Name }) {
        $f = Get-ChildItem -LiteralPath $d.FullName -Recurse -File -Include *.rs, Cargo.toml -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($f -and $f.LastWriteTime -gt $newest) { $newest = $f.LastWriteTime }
    }
    $root = Get-Item -LiteralPath (Join-Path $RepoRoot 'Cargo.toml') -ErrorAction SilentlyContinue
    if ($root -and $root.LastWriteTime -gt $newest) { $newest = $root.LastWriteTime }
    return $newest
}
