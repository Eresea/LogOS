param(
    [ValidateSet('all', 'host', 'uefi', 'services')]
    [string]$Stage = 'all',
    [switch]$Release
)

$ErrorActionPreference = 'Stop'

if ($Stage -in @('all', 'host')) {
    Write-Host '== format =='
    cargo fmt --check

    Write-Host '== clippy =='
    cargo clippy --lib -- -D warnings

    Write-Host '== host tests =='
    cargo test --lib
}

if ($Stage -in @('all', 'uefi')) {
    Write-Host '== UEFI build =='
    $args = @('build', '--target', 'x86_64-unknown-uefi')
    if ($Release) { $args += '--release' }
    cargo @args

    Write-Host '== UEFI clippy =='
    cargo clippy --target x86_64-unknown-uefi -- -D warnings

    Write-Host '== UEFI proof build =='
    cargo build --features qemu-proof --target x86_64-unknown-uefi
}

if ($Stage -in @('all', 'services')) {
    Write-Host '== service ELF images =='
    .\scripts\build-services.ps1 -Release
    cargo clippy --target x86_64-unknown-none -p logos-service-images --bins -- -D warnings
}
