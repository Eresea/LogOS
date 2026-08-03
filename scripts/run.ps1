param([switch]$Release, [switch]$Headless, [switch]$Monitor)

$repoRoot = Split-Path $PSScriptRoot -Parent
$profile = if ($Release) { "release" } else { "debug" }
$efi = Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-uefi.efi"
$esp = Join-Path $repoRoot "target\esp"
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { "C:\Program Files\qemu\qemu-system-x86_64.exe" }
if (-not (Test-Path $qemuPath)) { throw "Install QEMU or add qemu-system-x86_64 to PATH." }

$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
if (-not (Test-Path $ovmf)) { throw "Set OVMF_CODE to an EDK2/OVMF firmware file." }

cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --package logos-uefi --target x86_64-unknown-uefi $(if ($Release) { "--release" })
cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --package logos-terminal-service --target x86_64-unknown-uefi $(if ($Release) { "--release" })
cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --package logos-sessions-service --target x86_64-unknown-uefi $(if ($Release) { "--release" })
cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --package logos-storage-service --target x86_64-unknown-uefi $(if ($Release) { "--release" })
cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --package logos-network-service --target x86_64-unknown-uefi $(if ($Release) { "--release" })
cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --package logos-gateway-service --target x86_64-unknown-uefi $(if ($Release) { "--release" })
New-Item -ItemType Directory -Force "$esp\EFI\BOOT" | Out-Null
New-Item -ItemType Directory -Force "$esp\EFI\LOGOS" | Out-Null
Copy-Item $efi "$esp\EFI\BOOT\BOOTX64.EFI" -Force
Copy-Item (Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-terminal-service.efi") "$esp\EFI\LOGOS\TERMINAL.EFI" -Force
Copy-Item (Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-sessions-service.efi") "$esp\EFI\LOGOS\SESSIONS.EFI" -Force
Copy-Item (Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-storage-service.efi") "$esp\EFI\LOGOS\STORAGE.EFI" -Force
Copy-Item (Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-network-service.efi") "$esp\EFI\LOGOS\NETWORK.EFI" -Force
Copy-Item (Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-gateway-service.efi") "$esp\EFI\LOGOS\GATEWAY.EFI" -Force
$disk = Join-Path $repoRoot "target\logos-store.raw"
if (-not (Test-Path $disk)) {
    $stream = [System.IO.File]::Create($disk)
    $stream.SetLength(16MB)
    $stream.Dispose()
}
$qemuArgs = @(
    '-machine', 'q35', '-m', '256M',
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$((Resolve-Path $esp).Path)",
    '-device', 'virtio-balloon-pci,disable-modern=on,id=logos-virtio',
    '-netdev', 'user,id=logos-net,net=10.0.2.0/24,dhcpstart=10.0.2.15',
    '-device', 'virtio-net-pci,disable-modern=on,netdev=logos-net,id=logos-network,mac=52:54:00:12:34:56',
    '-drive', "if=none,format=raw,cache=writeback,file=$disk,id=logos-store",
    '-device', 'virtio-blk-pci,disable-modern=on,drive=logos-store,id=logos-block',
    '-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9'
)
if ($Headless) { $qemuArgs += @('-display', 'none') }
if ($Monitor) { $qemuArgs += @('-monitor', 'tcp:127.0.0.1:4444,server,nowait') }
Write-Host "Booting LogOS; debug output follows (Ctrl+C to stop QEMU)."
& $qemuPath @qemuArgs
