param(
    [switch]$Release,
    [switch]$Proof,
    [switch]$PackageProof,
    [switch]$StorageProof,
    [switch]$FetchProof
)

$ErrorActionPreference = 'Stop'

$installedTargets = rustup target list --installed
if ($installedTargets -notcontains 'x86_64-unknown-none') {
    throw 'x86_64-unknown-none is required; install it with rustup target add x86_64-unknown-none'
}

$buildArgs = @(
    'build', '--target', 'x86_64-unknown-none',
    '-p', 'logos-service-images', '--bins'
)
$features = @()
if ($Proof) { $features += 'qemu-proof' }
if ($PackageProof) { $features += 'package-proof' }
if ($StorageProof) { $features += 'storage-proof' }
if ($FetchProof) { $features += 'fetch-proof' }
if ($features.Count -gt 0) { $buildArgs += @('--features', ($features -join ',')) }
if ($Release) { $buildArgs += '--release' }

$env:CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS = '-C relocation-model=static -C code-model=large -C link-arg=--image-base=0x10000000000 -C link-arg=--no-pie'
cargo @buildArgs
if ($LASTEXITCODE -ne 0) { throw "Service image build failed with exit code $LASTEXITCODE." }
$userBuildArgs = @(
    'build', '--target', 'x86_64-unknown-none', '-p', 'logos-service-images', '--bin', 'logos-user',
    '--features', 'user-kdf'
)
if ($Release) { $userBuildArgs += '--release' }
cargo @userBuildArgs
if ($LASTEXITCODE -ne 0) { throw "User service image build failed with exit code $LASTEXITCODE." }

$profile = if ($Release) { 'release' } else { 'debug' }
$output = Join-Path $PSScriptRoot '..\build\esp\EFI\LOGOS'
New-Item -ItemType Directory -Force -Path $output | Out-Null

$names = @('INPUT', 'DISPLAY', 'TERMINAL', 'SESSION', 'FLOW', 'STORAGE', 'NETWORK', 'FETCH', 'DEVICE', 'USER')
foreach ($name in $names) {
    $source = Join-Path $PSScriptRoot "..\target\x86_64-unknown-none\$profile\logos-$($name.ToLower())"
    $destination = Join-Path $output "$name.ELF"
    Copy-Item -LiteralPath $source -Destination $destination -Force
    $file = Get-Item -LiteralPath $destination
    if ($file.Length -eq 0 -or $file.Length -gt 512KB) {
        throw "$name service image exceeds the fixed 512 KiB bound"
    }
    $magic = [System.IO.File]::ReadAllBytes($destination)[0..3]
    if ([BitConverter]::ToString($magic) -ne '7F-45-4C-46') {
        throw "$name service image is not ELF"
    }
}
