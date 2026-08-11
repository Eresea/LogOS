param([switch]$Release, [switch]$Headless)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$profile = if ($Release) { 'release' } else { 'debug' }
$efi = Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-vnext.efi"
$esp = Join-Path $repoRoot 'target\esp'
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { 'C:\Program Files\qemu\share\edk2-x86_64-code.fd' }

if (-not (Test-Path $qemuPath)) { throw 'Install QEMU or add qemu-system-x86_64 to PATH.' }
if (-not (Test-Path $ovmf)) { throw 'Set OVMF_CODE to an OVMF firmware file.' }

$buildArgs = @('build', '--target', 'x86_64-unknown-uefi')
if ($Release) { $buildArgs += '--release' }
cargo @buildArgs

New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\BOOT') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force

$qemuArgs = @(
    '-machine', 'q35', '-m', '128M',
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$((Resolve-Path $esp).Path)",
    '-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9'
)
if ($Headless) { $qemuArgs += @('-display', 'none') }

& $qemuPath @qemuArgs
