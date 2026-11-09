set windows-shell := ["powershell.exe", "-NoProfile", "-Command"]

default:
    @just --list

check:
    $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"; cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"; cargo test --workspace

fmt-check:
    $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"; cargo fmt --all --check

# Regenerate docs/TOOLS.md from the tool catalog (LF, no BOM).
docs:
    $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"; $md = cargo run -q -p deskcraft-mcp -- tools --markdown; [IO.File]::WriteAllText((Join-Path (Get-Location) 'docs\TOOLS.md'), (($md -join "`n").TrimEnd() + "`n"), (New-Object Text.UTF8Encoding $false))

run-mcp:
    $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"; cargo run -p deskcraft-mcp -- mcp

build-release:
    $env:Path = "$env:USERPROFILE\\.cargo\\bin;$env:Path"; cargo build --release --workspace

ci: check test fmt-check
