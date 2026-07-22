$ErrorActionPreference = 'Stop'

Set-Location (Split-Path $PSScriptRoot -Parent)
cargo fmt --check
cargo clippy --target x86_64-unknown-uefi -- -D warnings
& "$PSScriptRoot\verify.ps1"
