param(
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

cargo test -p logos-storage-service program_package_is_persistent_and_name_keyed
cargo test -p logos-vnext program_handles_are_generation_safe_and_name_bound
cargo test -p logos-flow typed_registry_covers_canonical_namespaces

Write-Host 'Persistent program proof PASS (install, durable name lookup, manager generation safety, typed Flow registry)'
