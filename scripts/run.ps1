param([switch]$Release)

$profile = if ($Release) { "release" } else { "debug" }
$efi = "target\x86_64-unknown-uefi\$profile\logos.efi"
$esp = "target\esp"
$qemu = Get-Command qemu-system-x86_64 -ErrorAction Stop
$ovmf = $env:OVMF_CODE
if (-not $ovmf) { throw "Set OVMF_CODE to your OVMF firmware file." }

cargo build --target x86_64-unknown-uefi $(if ($Release) { "--release" })
New-Item -ItemType Directory -Force "$esp\EFI\BOOT" | Out-Null
Copy-Item $efi "$esp\EFI\BOOT\BOOTX64.EFI" -Force
& $qemu.Source -machine q35 -m 256M -bios $ovmf -drive "format=raw,file=fat:rw:$((Resolve-Path $esp).Path)" -debugcon stdio -global isa-debugcon.iobase=0xe9
