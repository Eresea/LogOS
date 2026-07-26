param([switch]$Release, [switch]$Headless)

$repoRoot = Split-Path $PSScriptRoot -Parent
$profile = if ($Release) { "release" } else { "debug" }
$efi = Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-uefi.efi"
$esp = Join-Path $repoRoot "target\esp"
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { "C:\Program Files\qemu\qemu-system-x86_64.exe" }
if (-not (Test-Path $qemuPath)) { throw "Install QEMU or add qemu-system-x86_64 to PATH." }

$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { "C:\Program Files\qemu\share\edk2-x86_64-code.fd" }
if (-not (Test-Path $ovmf)) { throw "Set OVMF_CODE to an EDK2/OVMF firmware file." }

cargo build --manifest-path (Join-Path $repoRoot "Cargo.toml") --target x86_64-unknown-uefi $(if ($Release) { "--release" })
New-Item -ItemType Directory -Force "$esp\EFI\BOOT" | Out-Null
Copy-Item $efi "$esp\EFI\BOOT\BOOTX64.EFI" -Force
$qemuArgs = @(
    '-machine', 'q35', '-m', '256M',
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$((Resolve-Path $esp).Path)",
    '-device', 'virtio-balloon-pci,disable-modern=on,id=logos-virtio',
    '-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9'
)
if ($Headless) { $qemuArgs += @('-display', 'none') }
Write-Host "Booting LogOS; debug output follows (Ctrl+C to stop QEMU)."
& $qemuPath @qemuArgs
