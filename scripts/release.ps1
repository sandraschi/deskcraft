#Requires -Version 5.1
<#
.SYNOPSIS
    Craft app release: gates, release build, smoke, plain exes + per-user NSIS installer
    (+ mcpb), launcher zip as an extra, SHA256SUMS. Publish is opt-in.

.DESCRIPTION
    VENDORED from mcp-central-docs\templates\craft-app (identical in every craft repo); per-repo
    settings are in scripts\craft.config.psd1. Standard:
    mcp-central-docs\standards\rust-artcraft\CRAFT_START_AND_RELEASE.md.

    Executables ship as plain release assets under stable names (never a zip as the only
    download). By default this only BUILDS and STAGES dist\. Nothing is tagged, pushed or
    uploaded unless -Publish is given. Stops at the first failed phase.

.PARAMETER SkipGates     Skip fmt, clippy and tests (they must have been run just before).
.PARAMETER SkipMcpb      Skip the mcpb bundle.
.PARAMETER SkipInstaller Skip the NSIS installer.
.PARAMETER AllowDirty    Build from a dirty tree (dry runs only; refused with -Publish).
.PARAMETER Publish       Tag, push the tag, create the GitHub release (notes: CHANGELOG_LATEST.md),
                         upload the artifacts. Needs gh, a clean trunk, everything pushed.
#>
param(
    [switch]$SkipGates,
    [switch]$SkipMcpb,
    [switch]$SkipInstaller,
    [switch]$AllowDirty,
    [switch]$Publish
)

# Native tools (cargo, git, makensis, bunx) are judged by exit code. EAP stays Continue: Windows
# PowerShell 5.1 turns their stderr progress into error records when output is redirected, and
# Stop would abort on the first "Compiling" line. Cmdlets that must not fail use -ErrorAction Stop.
$ErrorActionPreference = 'Continue'
$RepoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'craft-common.ps1')
Initialize-CraftCargo
$cfg = Get-CraftConfig $RepoRoot
$Name = $cfg.Name
$Dist = Join-Path $RepoRoot 'dist'

function Write-Phase { param([string]$m) Write-Host ''; Write-Host "=== $m ===" -ForegroundColor Cyan }
function Write-Ok { param([string]$m) Write-Host "  [ok] $m" -ForegroundColor Green }
function Write-Info { param([string]$m) Write-Host "  [..] $m" -ForegroundColor Gray }
function Fail { param([string]$m) Write-Host "  [FAIL] $m" -ForegroundColor Red; throw $m }
function Need { param([int]$code, [string]$step) if ($code -ne 0) { Fail "$step exited with code $code" } }

# ---- Phase 0: preflight ------------------------------------------------------
Write-Phase 'Phase 0: preflight'
Set-Location -LiteralPath $RepoRoot
$mainPkg = if ($cfg.AppPackage) { $cfg.AppPackage } else { $cfg.CliPackage }
if (-not $mainPkg) { Fail 'craft.config.psd1 names neither AppPackage nor CliPackage' }
$Version = Get-CraftVersion $RepoRoot $mainPkg
if (-not $Version) { Fail "version of package $mainPkg not found (cargo metadata)" }
$Tag = "v$Version"
Write-Ok "Version $Version, tag $Tag"

if ($Publish -and $AllowDirty) { Fail '-AllowDirty is for dry runs; it cannot be combined with -Publish' }
$dirty = git status --porcelain
if ($dirty -and -not $AllowDirty) { Fail "working tree is dirty:`n$dirty" }
if ($dirty) { Write-Info 'dirty tree allowed (dry run)' } else { Write-Ok 'tree clean' }

$Trunk = ((git symbolic-ref --short refs/remotes/origin/HEAD 2>$null) -replace '^origin/', '').Trim()
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if (-not $Trunk) { if (@('main', 'master') -contains $branch) { $Trunk = $branch } else { $Trunk = 'main' } }
if ($branch -ne $Trunk) { Fail "on branch '$branch', releases are cut from $Trunk" }
$extra = git branch --format='%(refname:short)' | Where-Object { $_ -ne $Trunk }
if ($extra) { Fail "extra local branches exist: $($extra -join ', '). Merge or delete them first (single trunk)" }
Write-Ok "on $Trunk, no stray branches"

if (git tag --list $Tag) {
    if ($Publish) { Fail "tag $Tag already exists locally; bump the version" }
    Write-Info "tag $Tag already exists (dry run continues; bump the version before -Publish)"
}
$latest = Join-Path $RepoRoot 'CHANGELOG_LATEST.md'
if ($Publish) {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Fail 'gh CLI not found' }
    if (git ls-remote --tags origin "refs/tags/$Tag") { Fail "tag $Tag already exists on the remote" }
    git fetch -q origin $Trunk 2>$null; Need $LASTEXITCODE "git fetch origin $Trunk"
    $ahead = git rev-list --count "origin/$Trunk..HEAD"
    if ($ahead -ne '0') { Fail "$ahead local commit(s) not pushed; push $Trunk first" }
    if (-not (Test-Path $latest)) { Fail 'CHANGELOG_LATEST.md missing (release notes of this version only)' }
    if ((Get-Content $latest -TotalCount 1) -notmatch [regex]::Escape($Version)) { Fail "CHANGELOG_LATEST.md does not start with version $Version" }
} elseif (-not (Test-Path $latest)) {
    Write-Info 'CHANGELOG_LATEST.md missing: needed before -Publish'
}
$makensis = @('C:\Program Files (x86)\NSIS\makensis.exe', 'C:\Program Files\NSIS\makensis.exe') | Where-Object { Test-Path $_ } | Select-Object -First 1
$wantInstaller = $cfg.Installer -and $cfg.AppBin -and -not $SkipInstaller
if ($wantInstaller -and -not $makensis) { Fail 'makensis not found (install NSIS or pass -SkipInstaller)' }
Write-Ok 'preflight done'

# ---- Phase 1: gates ----------------------------------------------------------
if (-not $SkipGates) {
    Write-Phase 'Phase 1: gates'
    cargo fmt --all --check; Need $LASTEXITCODE 'fmt'
    cargo clippy --workspace --all-targets --all-features -- -D warnings; Need $LASTEXITCODE 'clippy'
    cargo test --workspace; Need $LASTEXITCODE 'tests'
    Write-Ok 'fmt, clippy, tests green'
}

# ---- Phase 2: release build + smoke -----------------------------------------
Write-Phase 'Phase 2: release build and smoke'
$targetDir = Get-CraftTargetDir $RepoRoot
$binDir = Join-Path $targetDir 'release'
$exes = @()
foreach ($pair in @(@($cfg.AppPackage, $cfg.AppBin), @($cfg.CliPackage, $cfg.CliBin))) {
    if (-not $pair[1]) { continue }
    $busy = Get-Process -Name $pair[1] -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $binDir "$($pair[1]).exe") }
    if ($busy) { Fail "$($pair[1]).exe is running from $binDir (PID $($busy[0].Id)); close it, it blocks the release build" }
    Need (Invoke-CraftBuild -Package $pair[0] -Bin $pair[1] -Release) "release build of $($pair[1])"
    $exe = Join-Path $binDir "$($pair[1]).exe"
    if (-not (Test-Path $exe)) { Fail "missing build output: $exe" }
    $exes += $exe
}
$smokeDb = Join-Path ([IO.Path]::GetTempPath()) "$Name-release-smoke-$PID.db"
$smoked = @()
if ($cfg.AppBin -and @($cfg.AppSmoke).Count -gt 0) {
    & (Join-Path $binDir "$($cfg.AppBin).exe") @(Expand-CraftArgs $cfg.AppSmoke $smokeDb) | Out-Null; Need $LASTEXITCODE "$($cfg.AppBin) smoke"
    $smoked += 'app'
}
if ($cfg.CliBin -and @($cfg.CliSmoke).Count -gt 0) {
    & (Join-Path $binDir "$($cfg.CliBin).exe") @(Expand-CraftArgs $cfg.CliSmoke $smokeDb) | Out-Null; Need $LASTEXITCODE "$($cfg.CliBin) smoke"
    $smoked += 'cli'
}
if ($cfg.CliBin -and @($cfg.McpSmoke).Count -gt 0) {
    $init = '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"release-smoke","version":"0"}}}'
    $resp = $init | & (Join-Path $binDir "$($cfg.CliBin).exe") @(Expand-CraftArgs $cfg.McpSmoke $smokeDb) 2>$null | Select-Object -First 1
    if ($resp -notmatch '"result"') { Fail "MCP initialize returned no result: $resp" }
    $smoked += 'MCP initialize'
}
Remove-Item "$smokeDb*" -Force -ErrorAction SilentlyContinue  # best effort
if ($smoked.Count -eq 0) { Write-Info 'no smoke configured (scripts\craft.config.psd1): build only' } else { Write-Ok "smoke: $($smoked -join ', ')" }

# ---- Phase 3: stage artifacts -----------------------------------------------
Write-Phase 'Phase 3: artifacts'
if (Test-Path $Dist) { Remove-Item $Dist -Recurse -Force -ErrorAction Stop }
New-Item -ItemType Directory -Force $Dist -ErrorAction Stop | Out-Null
# Plain executables under stable names: releases/latest/download/<name>.exe is a fixed URL.
$assets = @()
foreach ($f in $exes) {
    $dst = Join-Path $Dist (Split-Path -Leaf $f)
    Copy-Item $f $dst -ErrorAction Stop
    Write-Ok "$(Split-Path -Leaf $f): $([math]::Round((Get-Item $dst).Length / 1MB, 1)) MB"
    $assets += $dst
}
if ($wantInstaller) {
    $setup = Join-Path $Dist "$Name-$Version-setup.exe"
    $defs = @("/DPRODUCT_NAME=$($cfg.Display)", "/DPRODUCT_ID=$Name", "/DPRODUCT_VERSION=$Version",
        "/DAPP_EXE=$($cfg.AppBin).exe", "/DBIN_DIR=$binDir", "/DSRC_DIR=$RepoRoot",
        "/DOUT_FILE=$setup", "/DWEB_SITE=$($cfg.Repository)")
    if ($cfg.CliBin) { $defs += "/DCLI_EXE=$($cfg.CliBin).exe" }
    & $makensis /V2 @defs (Join-Path $RepoRoot 'installer\craft-installer.nsi'); Need $LASTEXITCODE 'makensis'
    if (-not (Test-Path $setup)) { Fail "makensis reported success but $setup is missing" }
    Write-Ok "installer: $(Split-Path -Leaf $setup), $([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB"
    $assets += $setup
}
if ($cfg.LauncherZip) {
    # Launcher-only extra (craft-launcher discovery): the same exes, never the sole download.
    $zip = Join-Path $Dist "$Name-$Version-windows-x64-portable.zip"
    Compress-Archive -Path $exes -DestinationPath $zip -Force -ErrorAction Stop
    Write-Ok "launcher zip (extra): $(Split-Path -Leaf $zip)"
    $assets += $zip
}
$manifestPath = Join-Path $RepoRoot 'mcpb\manifest.json'
if ($cfg.Mcpb -and $cfg.CliBin -and -not $SkipMcpb) {
    if (-not (Test-Path $manifestPath)) { Fail 'Mcpb = $true but mcpb\manifest.json is missing' }
    $bunx = Join-Path $env:USERPROFILE '.bun\bin\bunx.exe'
    if (-not (Test-Path $bunx)) { $g = Get-Command bunx -ErrorAction SilentlyContinue; if ($g) { $bunx = $g.Source } else { Fail 'bunx not found' } }
    $mstage = Join-Path $Dist 'mcpb-stage'
    New-Item -ItemType Directory -Force $mstage -ErrorAction Stop | Out-Null
    $manifest = (Get-Content $manifestPath -Raw -ErrorAction Stop) -replace '__VERSION__', $Version
    [IO.File]::WriteAllText((Join-Path $mstage 'manifest.json'), $manifest, (New-Object Text.UTF8Encoding $false))
    Copy-Item (Join-Path $binDir "$($cfg.CliBin).exe") $mstage -ErrorAction Stop
    $mcpb = Join-Path $Dist "$Name-v$Version.mcpb"
    & $bunx '@anthropic-ai/mcpb' validate (Join-Path $mstage 'manifest.json'); Need $LASTEXITCODE 'mcpb validate manifest'
    & $bunx '@anthropic-ai/mcpb' pack $mstage $mcpb; Need $LASTEXITCODE 'mcpb pack'
    Remove-Item $mstage -Recurse -Force -ErrorAction Stop
    if (-not (Test-Path $mcpb)) { Fail "mcpb pack reported success but $mcpb is missing" }
    Write-Ok "mcpb: $([math]::Round((Get-Item $mcpb).Length / 1MB, 1)) MB"
    $assets += $mcpb
}

$sums = foreach ($a in $assets) { "{0}  {1}" -f (Get-FileHash $a -Algorithm SHA256 -ErrorAction Stop).Hash.ToLower(), (Split-Path -Leaf $a) }
$sumFile = Join-Path $Dist 'SHA256SUMS.txt'
[IO.File]::WriteAllText($sumFile, (($sums -join "`n") + "`n"), (New-Object Text.UTF8Encoding $false))
$assets += $sumFile
$sums | ForEach-Object { Write-Info $_ }

# ---- Phase 4: publish (opt-in) ----------------------------------------------
if ($Publish) {
    Write-Phase "Phase 4: publish $Tag"
    git tag $Tag; Need $LASTEXITCODE 'git tag'
    git push origin $Tag; Need $LASTEXITCODE 'git push tag'
    gh release create $Tag --title "$Name $Tag" --notes-file $latest; Need $LASTEXITCODE 'gh release create'
    foreach ($a in $assets) { gh release upload $Tag $a --clobber; Need $LASTEXITCODE "upload $(Split-Path -Leaf $a)" }
    Write-Ok "$($cfg.Repository)/releases/tag/$Tag"
} else {
    Write-Phase 'Done (not published)'
    Write-Info "Artifacts are in $Dist. Re-run with -Publish to tag, push and upload."
}
