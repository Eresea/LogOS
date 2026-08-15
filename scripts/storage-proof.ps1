param(
    [string]$DiskImage,
    [ValidateRange(16, 4096)]
    [int]$DiskMiB = 64,
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
if (-not $DiskImage) { $DiskImage = Join-Path $repoRoot 'target\storage-proof.raw' }
$disk = [System.IO.Path]::GetFullPath($DiskImage)
$target = Join-Path $repoRoot 'target'
$log = Join-Path $target 'storage-proof.log'
$efi = Join-Path $repoRoot 'target\x86_64-unknown-uefi\release\logos-vnext.efi'
$esp = Join-Path $repoRoot 'target\storage-proof-esp'
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { 'C:\Program Files\qemu\share\edk2-x86_64-code.fd' }

if (-not (Test-Path $qemuPath)) { throw 'Install QEMU or add qemu-system-x86_64 to PATH.' }
if (-not (Test-Path $ovmf)) { throw 'Set OVMF_CODE to an OVMF firmware file.' }

New-Item -ItemType Directory -Force $target | Out-Null
if (-not (Test-Path $disk)) {
    $stream = [System.IO.File]::Open($disk, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write)
    try { $stream.SetLength([int64]$DiskMiB * 1MB) } finally { $stream.Dispose() }
}

& cargo build --release --target x86_64-unknown-uefi
if ($LASTEXITCODE -ne 0) { throw 'UEFI build failed.' }
& (Join-Path $PSScriptRoot 'build-services.ps1') -Release
if ($LASTEXITCODE -ne 0) { throw 'Service image build failed.' }

New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\BOOT') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\LOGOS') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force
Copy-Item (Join-Path $repoRoot 'build\esp\EFI\LOGOS\*.ELF') (Join-Path $esp 'EFI\LOGOS') -Force

$espPath = ((Resolve-Path $esp).Path).Replace('\', '/')
$baseArgs = @(
    '-machine', 'q35', '-m', '128M', '-smp', '1',
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$espPath",
    '-drive', "if=none,id=storage-disk,format=raw,file=$disk",
    '-device', 'virtio-blk-pci,drive=storage-disk,disable-legacy=on',
    '-display', 'none', '-no-reboot',
    '-debugcon', "file:$log", '-global', 'isa-debugcon.iobase=0xe9'
)

function Invoke-StorageBoot {
    param([string]$ExpectedMarker)

    Remove-Item $log -Force -ErrorAction SilentlyContinue
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $qemuPath
    if ($psi.PSObject.Properties.Name -contains 'ArgumentList') {
        foreach ($argument in $baseArgs) { [void]$psi.ArgumentList.Add($argument) }
    } else {
        $psi.Arguments = (($baseArgs | ForEach-Object {
            '"' + $_.Replace('"', '\"') + '"'
        }) -join ' ')
    }
    $psi.UseShellExecute = $false
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $psi
    [void]$process.Start()
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            if (Test-Path $log) {
                $text = Get-Content $log -Raw
                if ($text -match [regex]::Escape($ExpectedMarker)) { return $true }
                if ($text -match '(?i)(?:FATAL|QEMU proof FAIL|panic)') { return $false }
            }
            if ($process.HasExited) { return $false }
            Start-Sleep -Milliseconds 250
        }
        return $false
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
        $process.Dispose()
    }
}

if (-not (Invoke-StorageBoot 'LogOS vNext: storage proof PASS')) {
    throw "Storage format/write/flush proof failed. Log: $log"
}
if (-not (Invoke-StorageBoot 'LogOS vNext: storage recovery PASS')) {
    throw "Storage reboot/recovery proof failed. Log: $log"
}
Write-Host 'Storage persistent-disk proof PASS'
