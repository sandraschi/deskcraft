#Requires -Version 5.1
<#
.SYNOPSIS
    Install the Windows app and the CLI for this craft.

.DESCRIPTION
    VENDORED from mcp-central-docs\templates\craft-app (identical in every craft repo).
    Copies the release app and CLI into %LOCALAPPDATA%\Programs\<Display>, adds that
    folder to the user PATH, and creates a Start Menu shortcut for the app.
    -RegisterClients also registers the installed MCP exe with the IDEs that are
    present (Cursor, Claude Desktop, Claude Code, Antigravity, Windsurf, OpenCode).
    -Clients limits that list. -Mcpb packs the Claude Desktop extension bundle when
    mcpb\manifest.json exists and installs it. A .mcpb is Claude Desktop only.
    -Uninstall removes the app, the Start Menu entries, the PATH entry, and the
    IDE config entries. -Check prints the plan and writes nothing.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-craft.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-craft.ps1 -RegisterClients
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-craft.ps1 -RegisterClients -Clients cursor,claude-desktop
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-craft.ps1 -Mcpb
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-craft.ps1 -Check
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-craft.ps1 -Uninstall
#>
param(
    [switch]$Check,
    [switch]$Uninstall,
    [switch]$RegisterClients,
    [string[]]$Clients,
    [switch]$Mcpb
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'craft-common.ps1')
Initialize-CraftCargo
$cfg = Get-CraftConfig $RepoRoot
$mcpBin = if ($cfg.McpBin) { $cfg.McpBin } else { $cfg.CliBin }
$mcpPkg = if ($cfg.McpPackage) { $cfg.McpPackage } else { $cfg.CliPackage }
if (-not $mcpBin -or -not $mcpPkg) {
    Write-Host "[$($cfg.Name)] craft.config.psd1 has no MCP or CLI binary" -ForegroundColor Red
    exit 1
}

$mcpArgs = @()
if ($cfg.McpArgs) {
    foreach ($arg in @($cfg.McpArgs)) { $mcpArgs += [string]$arg }
}

$pairs = @()
$seen = @{}
foreach ($item in @(
        @{ Bin = [string]$cfg.AppBin; Pkg = [string]$cfg.AppPackage },
        @{ Bin = [string]$cfg.CliBin; Pkg = [string]$cfg.CliPackage },
        @{ Bin = [string]$mcpBin; Pkg = [string]$mcpPkg })) {
    if (-not $item.Bin -or $seen.ContainsKey($item.Bin)) { continue }
    $seen[$item.Bin] = $true
    $pairs += $item
}

$targetDir = Get-CraftTargetDir $RepoRoot
$releaseDir = Join-Path $targetDir 'release'
$display = if ($cfg.Display) { [string]$cfg.Display } else { [string]$cfg.Name }
$installDir = Join-Path $env:LOCALAPPDATA "Programs\$display"
$programs = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$shortcutDir = Join-Path $programs $display
$shortcut = Join-Path $shortcutDir "$display.lnk"

function Find-BuiltExe([string]$Bin) {
    foreach ($candidate in @(
            (Join-Path $releaseDir "$Bin.exe"),
            (Join-Path $RepoRoot "dist\$Bin.exe"),
            (Join-Path (Join-Path $targetDir 'debug') "$Bin.exe"))) {
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }
    return $null
}

function Ensure-ReleaseExe($Pair) {
    $found = Find-BuiltExe $Pair.Bin
    $releaseExe = Join-Path $releaseDir "$($Pair.Bin).exe"
    if ($found -and $found -eq $releaseExe) { return $releaseExe }
    if ($found -and $Check) { return $found }
    if ($Check) {
        Write-Host "[$($cfg.Name)] would build $($Pair.Bin) (release)" -ForegroundColor Yellow
        return $releaseExe
    }
    if (-not $found) {
        $drive = New-Object System.IO.DriveInfo ([IO.Path]::GetPathRoot($targetDir))
        $freeGb = [math]::Round($drive.AvailableFreeSpace / 1GB, 1)
        if ($drive.AvailableFreeSpace -lt 8GB) {
            Write-Host "[$($cfg.Name)] $freeGb GB free on $($drive.Name.TrimEnd('\')); need 8 GB to build $($Pair.Bin)" -ForegroundColor Red
            exit 1
        }
        Set-Location -LiteralPath $RepoRoot
        Write-Host "[$($cfg.Name)] building $($Pair.Bin) (release)..." -ForegroundColor Cyan
        $code = Invoke-CraftBuild -Package $Pair.Pkg -Bin $Pair.Bin -Release
        if ($code -ne 0 -or -not (Test-Path -LiteralPath $releaseExe)) {
            Write-Host "[$($cfg.Name)] build failed (exit $code)" -ForegroundColor Red
            exit $(if ($code) { $code } else { 1 })
        }
        return $releaseExe
    }
    return $found
}

$built = @{}
foreach ($pair in $pairs) { $built[$pair.Bin] = Ensure-ReleaseExe $pair }

function Set-UserPath([string]$Dir, [bool]$Remove) {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $current) { $current = '' }
    $keep = @()
    $has = $false
    foreach ($part in ($current -split ';')) {
        if (-not $part) { continue }
        if ($part.TrimEnd('\') -eq $Dir.TrimEnd('\')) { $has = $true; continue }
        $keep += $part
    }
    if ($Remove) {
        if (-not $has) { return }
        [Environment]::SetEnvironmentVariable('Path', ($keep -join ';'), 'User')
        return
    }
    if ($has) { return }
    $keep += $Dir
    [Environment]::SetEnvironmentVariable('Path', ($keep -join ';'), 'User')
    $env:Path = $Dir + ';' + $env:Path
}

function Remove-Tree([string]$Path) {
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
}

if ($Uninstall) {
    Remove-Tree $shortcutDir
    Remove-Tree $installDir
    Set-UserPath $installDir $true
    Write-Host "[$($cfg.Name)] removed $installDir" -ForegroundColor Green
} elseif ($Check) {
    Write-Host "[$($cfg.Name)] app  -> $(Join-Path $installDir "$($cfg.AppBin).exe")"
    Write-Host "[$($cfg.Name)] cli  -> $installDir on the user PATH"
    Write-Host "[$($cfg.Name)] menu -> $shortcut"
} else {
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    foreach ($pair in $pairs) {
        $src = $built[$pair.Bin]
        if (-not (Test-Path -LiteralPath $src)) { continue }
        Copy-Item -LiteralPath $src -Destination (Join-Path $installDir "$($pair.Bin).exe") -Force
        Write-Host "[$($cfg.Name)] installed $($pair.Bin).exe" -ForegroundColor Green
    }
    if ($cfg.AppBin) {
        New-Item -ItemType Directory -Force -Path $shortcutDir | Out-Null
        $shell = New-Object -ComObject WScript.Shell
        $link = $shell.CreateShortcut($shortcut)
        $link.TargetPath = Join-Path $installDir "$($cfg.AppBin).exe"
        $link.WorkingDirectory = $installDir
        $link.Description = $display
        $link.Save()
        Write-Host "[$($cfg.Name)] Start Menu: $display" -ForegroundColor Green
    }
    Set-UserPath $installDir $false
    Write-Host "[$($cfg.Name)] CLI on user PATH: $installDir" -ForegroundColor Green
}

$allowedClients = @('claude-desktop', 'cursor', 'antigravity', 'windsurf', 'opencode', 'claude-code')
$clientList = @()
foreach ($item in @($Clients)) {
    if (-not $item) { continue }
    foreach ($part in ($item -split ',')) {
        $name = $part.Trim()
        if (-not $name) { continue }
        if ($allowedClients -notcontains $name) {
            Write-Host "[$($cfg.Name)] unknown client '$name'. Use: $($allowedClients -join ', ')" -ForegroundColor Red
            exit 1
        }
        if ($clientList -notcontains $name) { $clientList += $name }
    }
}
$wantClients = $RegisterClients -or $clientList.Count -gt 0

$mcpExe = Join-Path $installDir "$mcpBin.exe"
if ($Check -or $Uninstall -or -not (Test-Path -LiteralPath $mcpExe)) {
    $mcpExe = $built[$mcpBin]
}

if ($Uninstall -or $wantClients) {
    $writer = Join-Path (Split-Path -Parent $RepoRoot) 'mcp-central-docs\scripts\install-mcp-clients.ps1'
    if (-not (Test-Path -LiteralPath $writer)) {
        Write-Host "[$($cfg.Name)] MCP client installer not found: $writer" -ForegroundColor Red
        if ($wantClients -and -not $Uninstall) { exit 1 }
    } else {
        $params = @{ Name = [string]$cfg.Name; Command = $mcpExe }
        if ($mcpArgs.Count -gt 0) { $params.Arguments = $mcpArgs }
        if ($Check) { $params.DryRun = $true }
        if ($Uninstall) { $params.Uninstall = $true }
        if ($clientList.Count -gt 0) { $params.Clients = $clientList }
        & $writer @params
    }
} else {
    Write-Host "[$($cfg.Name)] IDE registration skipped (pass -RegisterClients)" -ForegroundColor Yellow
}

$mcpbScript = Join-Path (Split-Path -Parent $RepoRoot) 'mcp-central-docs\scripts\install-mcpb.ps1'
$manifest = Join-Path $RepoRoot 'mcpb\manifest.json'
if (-not $Mcpb) {
    if ($Uninstall -and $cfg.Mcpb -and (Test-Path -LiteralPath $manifest)) {
        Write-Host "[$($cfg.Name)] Claude Desktop bundle left installed. Remove it in Settings, Extensions." -ForegroundColor Yellow
    } else {
        Write-Host "[$($cfg.Name)] mcpb skipped (pass -Mcpb)" -ForegroundColor Yellow
    }
    exit 0
}
if (-not $cfg.Mcpb -or -not (Test-Path -LiteralPath $manifest)) {
    Write-Host "[$($cfg.Name)] no mcpb manifest; bundle skipped" -ForegroundColor Yellow
    exit 0
}
if ($Uninstall) {
    Write-Host "[$($cfg.Name)] Claude Desktop bundle left installed. Remove it in Settings, Extensions." -ForegroundColor Yellow
    exit 0
}
$dist = Join-Path $RepoRoot 'dist'
$bundle = $null
if (Test-Path -LiteralPath $dist) {
    $files = Get-ChildItem -LiteralPath $dist -Filter '*.mcpb' -File
    foreach ($file in $files) {
        if (-not $bundle -or $file.LastWriteTime -gt $bundle.LastWriteTime) { $bundle = $file }
    }
}
if (-not $bundle) {
    if ($Check) {
        Write-Host "[$($cfg.Name)] mcpb  -> would pack dist\$($cfg.Name).mcpb and install it in Claude Desktop" -ForegroundColor Yellow
        exit 0
    }
    $bunx = Join-Path $env:USERPROFILE '.bun\bin\bunx.exe'
    if (-not (Test-Path -LiteralPath $bunx)) {
        $cmd = Get-Command bunx -ErrorAction SilentlyContinue
        if ($cmd) { $bunx = $cmd.Source }
    }
    if (-not (Test-Path -LiteralPath $bunx)) {
        Write-Host "[$($cfg.Name)] mcpb skipped: bunx not found and dist has no bundle" -ForegroundColor Yellow
        exit 0
    }
    $version = Get-CraftVersion $RepoRoot $(if ($cfg.AppPackage) { $cfg.AppPackage } else { $cfg.CliPackage })
    if (-not $version) { $version = '0.0.0' }
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    $stage = Join-Path $dist 'mcpb-stage'
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    $text = ([IO.File]::ReadAllText($manifest)) -replace '__VERSION__', $version
    [IO.File]::WriteAllText((Join-Path $stage 'manifest.json'), $text, (New-Object Text.UTF8Encoding $false))
    Copy-Item -LiteralPath $mcpExe -Destination $stage -Force
    $out = Join-Path $dist "$($cfg.Name)-v$version.mcpb"
    & $bunx '@anthropic-ai/mcpb' pack $stage $out
    Remove-Item -LiteralPath $stage -Recurse -Force
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $out)) {
        Write-Host "[$($cfg.Name)] mcpb pack failed" -ForegroundColor Yellow
        exit 0
    }
    $bundle = Get-Item -LiteralPath $out
}
if ($Check) {
    Write-Host "[$($cfg.Name)] mcpb  -> $($bundle.FullName) into Claude Desktop" -ForegroundColor Cyan
    exit 0
}
if (-not (Test-Path -LiteralPath $mcpbScript)) {
    Write-Host "[$($cfg.Name)] bundle is at $($bundle.FullName); Claude Desktop installer not found" -ForegroundColor Yellow
    exit 0
}
& $mcpbScript -Source $bundle.FullName
Write-Host "[$($cfg.Name)] mcpb installed. Quit Claude Desktop from the tray and open it again." -ForegroundColor Green
exit 0
