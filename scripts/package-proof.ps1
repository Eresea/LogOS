param(
    [switch]$Release,
    [ValidateRange(16, 4096)]
    [int]$DiskMiB = 64,
    [ValidateRange(1, 10)]
    [int]$TimeoutSeconds = 10
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$target = Join-Path $repoRoot 'target'
$profile = if ($Release) { 'release' } else { 'debug' }
$efi = Join-Path $target "x86_64-unknown-uefi\$profile\logos-vnext.efi"
$packageElf = Join-Path $repoRoot 'build\esp\EFI\LOGOS\INPUT.ELF'
$esp = Join-Path $target 'package-proof-esp'
$disk = Join-Path $target 'package-proof.raw'
$log = Join-Path $target 'package-proof.log'
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { 'C:\Program Files\qemu\share\edk2-x86_64-code.fd' }
if (-not (Test-Path $qemuPath)) { throw 'Install QEMU or add qemu-system-x86_64 to PATH.' }
if (-not (Test-Path $ovmf)) { throw 'Set OVMF_CODE to an OVMF firmware file.' }

$buildArgs = @('build', '--features', 'package-proof', '--target', 'x86_64-unknown-uefi')
if ($Release) { $buildArgs += '--release' }
cargo @buildArgs
& (Join-Path $PSScriptRoot 'build-services.ps1') -Release -Proof -PackageProof

if (-not (Test-Path $packageElf)) { throw 'Input ELF was not built.' }
New-Item -ItemType Directory -Force $esp | Out-Null
New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\BOOT') | Out-Null
New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\LOGOS') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force
Copy-Item (Join-Path $repoRoot 'build\esp\EFI\LOGOS\*.ELF') (Join-Path $esp 'EFI\LOGOS') -Force

cargo run -p logos-storage-service --bin package-seed -- $disk $packageElf
if (-not (Test-Path $disk)) { throw 'Package seed did not create a disk.' }

$espPath = ((Resolve-Path $esp).Path).Replace('\', '/')
$ovmfPath = ((Resolve-Path $ovmf).Path).Replace('\', '/')

function Invoke-PackageBoot {
    param([int]$BootNumber)

    Remove-Item $log -Force -ErrorAction SilentlyContinue
    $args = @(
        '-machine', 'q35', '-m', '128M', '-smp', '1',
        '-drive', "if=pflash,format=raw,readonly=on,file=$ovmfPath",
        '-drive', "format=raw,file=fat:rw:$espPath",
        '-drive', "if=none,id=storage-disk,format=raw,file=$disk,cache=writethrough",
        '-device', 'virtio-blk-pci,drive=storage-disk,disable-legacy=on',
        '-display', 'none', '-no-reboot', '-debugcon', "file:$log",
        '-global', 'isa-debugcon.iobase=0xe9'
    )
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $qemuPath
    $psi.Arguments = ($args | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + $_.Replace('"', '\"') + '"' } else { $_ }
    }) -join ' '
    $psi.UseShellExecute = $false
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $psi
    [void]$process.Start()
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        while ([DateTime]::UtcNow -lt $deadline) {
            if (Test-Path $log) {
                $text = Get-Content $log -Raw
                $complete = $text -match 'LogOS vNext: package activation PASS' `
                    -and $text -match 'LogOS vNext: corrupt package rollback PASS'
                if ($complete) {
                    Write-Host "Filesystem package boot $BootNumber PASS"
                    return
                }
                if ($text -match '(?i)(?:LogOS vNext: (?:FATAL|QEMU proof FAIL|panic)|FAULT)') {
                    throw "Package boot $BootNumber reported a fatal error. Log: $log"
                }
            }
            if ($process.HasExited) { throw "Package boot $BootNumber exited early with code $($process.ExitCode)." }
            Start-Sleep -Milliseconds 250
        }
        throw "Package boot $BootNumber timed out. Log: $log"
    } finally {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            [void]$process.WaitForExit(5000)
        }
        $process.Dispose()
    }
}

Invoke-PackageBoot 1
Invoke-PackageBoot 2
Write-Host 'Filesystem package proof PASS (including reopen)'
