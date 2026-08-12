param(
    [switch]$Release,
    [switch]$Headless,
    [switch]$Proof,
    [ValidateRange(1, 8)]
    [int]$Cpus = 1,
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path $PSScriptRoot -Parent
$profile = if ($Release) { 'release' } else { 'debug' }
$efi = Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-vnext.efi"
$esp = Join-Path $repoRoot 'target\esp'
$log = Join-Path $repoRoot "target\qemu-proof-$Cpus.log"
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { 'C:\Program Files\qemu\share\edk2-x86_64-code.fd' }

if (-not (Test-Path $qemuPath)) { throw 'Install QEMU or add qemu-system-x86_64 to PATH.' }
if (-not (Test-Path $ovmf)) { throw 'Set OVMF_CODE to an OVMF firmware file.' }

$buildArgs = @('build', '--target', 'x86_64-unknown-uefi')
if ($Proof) { $buildArgs += @('--features', 'qemu-proof') }
if ($Release) { $buildArgs += '--release' }
cargo @buildArgs

& (Join-Path $PSScriptRoot 'build-services.ps1') -Release

New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\BOOT') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force
New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\LOGOS') | Out-Null
Copy-Item (Join-Path $repoRoot 'build\esp\EFI\LOGOS\*.ELF') (Join-Path $esp 'EFI\LOGOS') -Force

$espPath = ((Resolve-Path $esp).Path).Replace('\', '/')
$qemuArgs = @(
    '-machine', 'q35', '-m', '128M', '-smp', $Cpus,
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$espPath",
    '-display', 'none'
)
if ($Proof) {
    Remove-Item $log -Force -ErrorAction SilentlyContinue
    $qemuArgs += @('-debugcon', "file:$log", '-global', 'isa-debugcon.iobase=0xe9')
} else {
    $qemuArgs += @('-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9')
    if (-not $Headless) { $qemuArgs = $qemuArgs | Where-Object { $_ -ne '-display' -and $_ -ne 'none' } }
}

if (-not $Proof) {
    & $qemuPath @qemuArgs
    exit $LASTEXITCODE
}

$psi = [Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $qemuPath
$psi.Arguments = ($qemuArgs | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + $_.Replace('"', '\"') + '"' } else { $_ }
    }) -join ' '
$psi.UseShellExecute = $false
$process = [Diagnostics.Process]::new()
$process.StartInfo = $psi
[void]$process.Start()

$deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
$passed = $false
while ([DateTime]::UtcNow -lt $deadline) {
    if (Test-Path $log) {
        $text = Get-Content $log -Raw
        if ($text -match 'LogOS vNext: QEMU proof PASS') {
            $passed = $true
            break
        }
        if ($text -match '(?i)(?:LogOS vNext: (?:FATAL|QEMU proof FAIL|panic)|FAULT)') {
            break
        }
    }
    if ($process.HasExited) { break }
    Start-Sleep -Milliseconds 250
}

if (-not $process.HasExited) { $process.Kill() }
$process.WaitForExit()
$result = if (Test-Path $log) { Get-Content $log -Raw } else { '' }
if (-not $passed) {
    if ($result) { Write-Host $result }
    throw "QEMU proof failed or timed out for -smp $Cpus. Log: $log"
}
Write-Host $result
