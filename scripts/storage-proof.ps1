param(
    [string]$DiskImage,
    [ValidateRange(16, 4096)]
    [int]$DiskMiB = 64,
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 60,
    [switch]$ResetDisk
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
if ($ResetDisk -and (Test-Path -LiteralPath $disk)) {
    Remove-Item -LiteralPath $disk -Force
}
if (-not (Test-Path $disk)) {
    $stream = [System.IO.File]::Open($disk, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write)
    try { $stream.SetLength([int64]$DiskMiB * 1MB) } finally { $stream.Dispose() }
    $marker = New-Object byte[] 4096
    [Text.Encoding]::ASCII.GetBytes('LOGOSBLK').CopyTo($marker, 0)
    $stream = [System.IO.File]::Open($disk, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Write)
    try { $stream.Write($marker, 0, $marker.Length) } finally { $stream.Dispose() }
}

& cargo build --release --features qemu-proof,storage-proof --target x86_64-unknown-uefi
if ($LASTEXITCODE -ne 0) { throw 'UEFI build failed.' }
& (Join-Path $PSScriptRoot 'build-services.ps1') -Release -Proof -StorageProof
if ($LASTEXITCODE -ne 0) { throw 'Service image build failed.' }

New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\BOOT') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\LOGOS') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force
Copy-Item (Join-Path $repoRoot 'build\esp\EFI\LOGOS\*.ELF') (Join-Path $esp 'EFI\LOGOS') -Force

$espPath = ((Resolve-Path $esp).Path).Replace('\', '/')
$baseArgs = @(
    '-machine', 'q35', '-m', '256M', '-smp', '1',
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$espPath",
    '-drive', "if=none,id=storage-disk,format=raw,file=$disk,cache=writethrough",
    '-device', 'virtio-blk-pci,drive=storage-disk,disable-legacy=on',
    '-display', 'none', '-no-reboot',
    '-debugcon', "file:$log", '-global', 'isa-debugcon.iobase=0xe9'
)

function Corrupt-InactiveV5Superblock {
    param([string]$Path)

    $blockBytes = 4096
    $blocks = @()
    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite)
    try {
        for ($slot = 0; $slot -lt 2; $slot++) {
            $bytes = New-Object byte[] $blockBytes
            $stream.Position = [int64]$slot * $blockBytes
            [void]$stream.Read($bytes, 0, $bytes.Length)
            if ([Text.Encoding]::ASCII.GetString($bytes, 0, 8) -eq 'LOGOSCOW' -and
                [BitConverter]::ToUInt16($bytes, 8) -eq 5) {
                $blocks += [pscustomobject]@{
                    Slot = $slot
                    Bytes = $bytes
                    Generation = [BitConverter]::ToUInt64($bytes, 16)
                }
            }
        }
        if ($blocks.Count -ne 2) { throw 'Expected two v5 storage superblocks.' }
        $active = $blocks | Sort-Object Generation -Descending | Select-Object -First 1
        $torn = New-Object byte[] $blockBytes
        $stream.Position = [int64](1 - $active.Slot) * $blockBytes
        $stream.Write($torn, 0, $torn.Length)
        $stream.Flush()
    } finally {
        $stream.Dispose()
    }
}

function Assert-V5UserCatalog {
    param([string]$Path)

    $blockBytes = 4096
    $stream = [System.IO.File]::Open($Path, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read)
    try {
        $roots = @()
        for ($slot = 0; $slot -lt 2; $slot++) {
            $bytes = New-Object byte[] $blockBytes
            $stream.Position = [int64]$slot * $blockBytes
            [void]$stream.Read($bytes, 0, $bytes.Length)
            if ([Text.Encoding]::ASCII.GetString($bytes, 0, 8) -eq 'LOGOSCOW' -and
                [BitConverter]::ToUInt16($bytes, 8) -eq 5) {
                $roots += [pscustomobject]@{
                    Generation = [BitConverter]::ToUInt64($bytes, 16)
                    CatalogStart = [BitConverter]::ToUInt64($bytes, 40)
                    CatalogBlocks = [BitConverter]::ToUInt32($bytes, 48)
                    CatalogBytes = [BitConverter]::ToUInt32($bytes, 52)
                    SystemStart = [BitConverter]::ToUInt64($bytes, 80)
                    SystemEnd = [BitConverter]::ToUInt64($bytes, 88)
                }
            }
        }
        if ($roots.Count -ne 2) { throw 'Expected two valid v5 roots.' }
        $root = $roots | Sort-Object Generation -Descending | Select-Object -First 1
        if ($root.CatalogBlocks -eq 0 -or $root.CatalogBytes -eq 0 -or
            $root.CatalogStart -lt $root.SystemStart -or
            ($root.CatalogStart + $root.CatalogBlocks) -gt $root.SystemEnd -or
            $root.CatalogBytes -gt ($root.CatalogBlocks * $blockBytes)) {
            throw 'v5 User catalog is outside the system pool.'
        }
        $catalog = New-Object byte[] 10
        $stream.Position = [int64]$root.CatalogStart * $blockBytes
        [void]$stream.Read($catalog, 0, $catalog.Length)
        if ([Text.Encoding]::ASCII.GetString($catalog, 0, 8) -ne 'LOGUSR01' -or
            [BitConverter]::ToUInt16($catalog, 8) -ne 1) {
            throw 'v5 User catalog snapshot header is invalid.'
        }
    } finally {
        $stream.Dispose()
    }
}

function Invoke-StorageBoot {
    param([string[]]$ExpectedMarkers)

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
                $allMarkersFound = $true
                foreach ($marker in $ExpectedMarkers) {
                    if ([string]::IsNullOrEmpty($text) -or -not ($text -match [regex]::Escape($marker))) {
                        $allMarkersFound = $false
                        break
                    }
                }
                if ($allMarkersFound) {
                    $postMarkerDeadline = [DateTime]::UtcNow.AddSeconds(5)
                    while ([DateTime]::UtcNow -lt $postMarkerDeadline) {
                        if (Test-Path $log) {
                            $postText = Get-Content $log -Raw
                            if ($postText -match '(?i)(?:FATAL|QEMU proof FAIL|storage command API FAIL|panic)') {
                                return $false
                            }
                        }
                        if ($process.HasExited) { return $true }
                        Start-Sleep -Milliseconds 250
                    }
                    return $true
                }
                if ($text -match '(?i)(?:FATAL|QEMU proof FAIL|storage command API FAIL|panic)') {
                    return $false
                }
            }
            if ($process.HasExited) { return $false }
            Start-Sleep -Milliseconds 250
        }
        return $false
    } finally {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            [void]$process.WaitForExit(5000)
        }
        if (-not $process.HasExited) { throw 'QEMU did not exit before disk reuse.' }
        Start-Sleep -Milliseconds 100
        $process.Dispose()
    }
}

if (-not (Invoke-StorageBoot -ExpectedMarkers @(
        'LogOS vNext: QEMU proof PASS',
        'LogOS vNext: storage proof PASS',
        'LogOS vNext: storage command API PASS',
        'LogOS vNext: storage command API cleanup PASS'
    ))) {
    throw "Storage format/write/flush proof failed. Log: $log"
}
if (Test-Path -LiteralPath $disk) {
    $diskStream = [System.IO.File]::Open($disk, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite)
    try { $diskStream.Flush($true) } finally { $diskStream.Dispose() }
}
Assert-V5UserCatalog $disk
Corrupt-InactiveV5Superblock $disk
if (-not (Invoke-StorageBoot -ExpectedMarkers @(
        'LogOS vNext: storage recovery PASS',
        'LogOS vNext: storage command API recovery PASS'
    ))) {
    throw "Storage reboot/recovery proof failed. Log: $log"
}
Assert-V5UserCatalog $disk
Write-Host 'Storage persistent-disk proof PASS'
