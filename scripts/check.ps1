param(
    [ValidateSet('all', 'host', 'uefi', 'services')]
    [string]$Stage = 'all',
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'native.ps1')

if ($Stage -in @('all', 'host')) {
    Write-Host '== format =='
    Invoke-Native { cargo fmt --check }

    Write-Host '== clippy =='
    Invoke-Native { cargo clippy --workspace --all-targets -- -D warnings }

    Write-Host '== host tests =='
    Invoke-Native { cargo test --workspace }
}

if ($Stage -in @('all', 'uefi')) {
    Write-Host '== UEFI build =='
    $buildArgs = @('build', '--target', 'x86_64-unknown-uefi')
    if ($Release) { $buildArgs += '--release' }
    Invoke-Native { cargo @buildArgs }

    Write-Host '== UEFI clippy =='
    Invoke-Native { cargo clippy --target x86_64-unknown-uefi -- -D warnings }

    Write-Host '== UEFI proof build =='
    Invoke-Native { cargo build --features qemu-proof --target x86_64-unknown-uefi }
}

if ($Stage -in @('all', 'services')) {
    Write-Host '== service ELF images =='
    .\scripts\build-services.ps1 -Release
    Invoke-Native { cargo clippy --target x86_64-unknown-none -p logos-service-images --bins -- -D warnings }
}
