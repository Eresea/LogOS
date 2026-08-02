param(
    [ValidateSet('all', 'host', 'uefi')]
    [string]$Stage = 'all',
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
Set-Location $repoRoot
$env:RUSTFLAGS = '-D warnings'

function Invoke-Checked([string]$Name, [scriptblock]$Command) {
    Write-Host "== $Name =="
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE" }
}

if ($Stage -in @('all', 'host')) {
    Invoke-Checked 'format' { cargo fmt --check --all }
    Invoke-Checked 'host clippy' {
        cargo clippy -p logos-abi -p logos-core -p logos-service-rt -p logos-store -p logos-storage-service -p logos-terminal --lib --all-features -- -D warnings
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo clippy -p logos-test --bin logos-test --all-features -- -D warnings
    }
    Invoke-Checked 'host tests' {
        cargo test -p logos-abi -p logos-core -p logos-service-rt -p logos-store -p logos-storage-service -p logos-terminal --lib --all-features
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        cargo test -p logos-test --bin logos-test
    }
    Invoke-Checked 'architecture' { python scripts/arch-deps.py --check }
    Invoke-Checked 'documentation links' { python scripts/docs-check.py }
    Invoke-Checked 'ADR index' { python scripts/adr-index.py --check }
}

if ($Stage -in @('all', 'uefi')) {
    $profile = if ($Release) { 'release' } else { 'debug' }
    $artifactRoot = Join-Path $repoRoot "target/logos-check/uefi/$profile"
    New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
    Get-ChildItem $artifactRoot -File |
        Where-Object { $_.Extension -eq '.efi' -or $_.Name -eq 'SHA256SUMS.txt' } |
        Remove-Item -Force
    $payloads = @()
    foreach ($package in @('logos-uefi', 'logos-terminal-service', 'logos-sessions-service', 'logos-storage-service', 'logos-network-service')) {
        $cargoArgs = @('build', '-p', $package, '--target', 'x86_64-unknown-uefi')
        if ($Release) { $cargoArgs += '--release' }
        Invoke-Checked "UEFI $package" { cargo @cargoArgs }
        $payload = Get-Item "target/x86_64-unknown-uefi/$profile/$package.efi"
        Copy-Item $payload -Destination $artifactRoot -Force
        $payloads += Get-Item (Join-Path $artifactRoot $payload.Name)
    }
    $payloads | Sort-Object Name | ForEach-Object {
        "{0}  {1}" -f (Get-FileHash $_ -Algorithm SHA256).Hash, $_.Name
    } | Set-Content (Join-Path $artifactRoot 'SHA256SUMS.txt')

    $espRoot = Join-Path $repoRoot "target/logos-check/esp/$profile"
    if (Test-Path $espRoot) {
        Get-ChildItem $espRoot -Recurse -File | Remove-Item -Force
    }
    New-Item -ItemType Directory -Force -Path (Join-Path $espRoot 'EFI/BOOT') | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $espRoot 'EFI/LOGOS') | Out-Null
    $espFiles = @(
        @{ source = 'logos-uefi.efi'; destination = 'EFI/BOOT/BOOTX64.EFI' },
        @{ source = 'logos-terminal-service.efi'; destination = 'EFI/LOGOS/TERMINAL.EFI' },
        @{ source = 'logos-sessions-service.efi'; destination = 'EFI/LOGOS/SESSIONS.EFI' },
        @{ source = 'logos-storage-service.efi'; destination = 'EFI/LOGOS/STORAGE.EFI' },
        @{ source = 'logos-network-service.efi'; destination = 'EFI/LOGOS/NETWORK.EFI' }
    )
    foreach ($file in $espFiles) {
        Copy-Item (Join-Path $artifactRoot $file.source) (Join-Path $espRoot $file.destination) -Force
    }
    $actual = @(Get-ChildItem $espRoot -Recurse -File | ForEach-Object {
        $_.FullName.Substring($espRoot.Length + 1).Replace('\', '/')
    } | Sort-Object)
    $expected = @($espFiles | ForEach-Object { $_.destination } | Sort-Object)
    if (@(Compare-Object $actual $expected).Count -ne 0) { throw 'ESP contents do not match the payload contract' }
    $expected | Set-Content (Join-Path $artifactRoot 'ESP-MANIFEST.txt')
}
