param(
    [switch]$Release,
    [switch]$Headless,
    [switch]$Interactive,
    [switch]$Proof,
    [ValidateRange(1, 8)]
    [int]$Cpus = 1,
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
if ($Interactive -and ($Headless -or $Proof)) { throw 'Choose exactly one of -Interactive, -Headless, or -Proof.' }
$interactiveMode = $Interactive -or (-not $Headless -and -not $Proof)
$repoRoot = Split-Path $PSScriptRoot -Parent
$profile = if ($Release) { 'release' } else { 'debug' }
$efi = Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-vnext.efi"
$esp = Join-Path $repoRoot 'target\esp'
$log = Join-Path $repoRoot "target\qemu-proof-$Cpus.log"
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { 'C:\Program Files\qemu\share\edk2-x86_64-code.fd' }
$qmpPort = 4444

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
    $qemuArgs += @('-debugcon', "file:$log", '-global', 'isa-debugcon.iobase=0xe9', '-qmp', "tcp:127.0.0.1:$qmpPort,server=on,wait=off")
} else {
    $qemuArgs += @('-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9')
    if ($interactiveMode) { $qemuArgs = $qemuArgs | Where-Object { $_ -ne '-display' -and $_ -ne 'none' } }
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

$result = if (Test-Path $log) { Get-Content $log -Raw } else { '' }
if (-not $passed) {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    $process.Dispose()
    if ($result) { Write-Host $result }
    throw "QEMU proof failed or timed out for -smp $Cpus. Log: $log"
}

function Invoke-QmpCommand {
    param(
        [System.IO.StreamWriter]$Writer,
        [System.IO.StreamReader]$Reader,
        [hashtable]$Command
    )
    $Writer.WriteLine(($Command | ConvertTo-Json -Compress -Depth 10))
    do {
        $read = $Reader.ReadLineAsync()
        if (-not $read.Wait(2000)) { throw 'QMP response timed out.' }
        $line = $read.Result
        if (-not $line) { throw 'QMP returned no response.' }
        $response = $line | ConvertFrom-Json
    } while ($response.event)
    if ($response.error) { throw "QMP command failed: $($response.error.desc)" }
    return $response
}

function Connect-Qmp {
    param([int]$Port)
    $client = [System.Net.Sockets.TcpClient]::new()
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $client.Connect('127.0.0.1', $Port)
            break
        } catch {
            Start-Sleep -Milliseconds 100
        }
    }
    if (-not $client.Connected) { throw 'QMP did not accept a connection.' }
    $stream = $client.GetStream()
    $stream.ReadTimeout = 2000
    $reader = [System.IO.StreamReader]::new($stream)
    $writer = [System.IO.StreamWriter]::new($stream)
    $writer.AutoFlush = $true
    $greeting = $reader.ReadLineAsync()
    if (-not $greeting.Wait(2000) -or -not $greeting.Result) { throw 'QMP greeting missing.' }
    Invoke-QmpCommand $writer $reader @{ execute = 'qmp_capabilities' } | Out-Null
    return @{ Client = $client; Reader = $reader; Writer = $writer }
}

$proofBefore = Join-Path $repoRoot "target\qemu-proof-before-$Cpus.ppm"
$proofAfter = Join-Path $repoRoot "target\qemu-proof-after-$Cpus.ppm"
Remove-Item $proofBefore, $proofAfter -Force -ErrorAction SilentlyContinue
$qmp = Connect-Qmp $qmpPort
try {
    Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $proofBefore } } | Out-Null
    foreach ($key in @('e', 'c', 'h', 'o', 'space', 'p', 'r', 'o', 'o', 'f', 'ret')) {
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{
            execute = 'human-monitor-command'
            arguments = @{ 'command-line' = "sendkey $key" }
        } | Out-Null
    }
    Start-Sleep -Milliseconds 500
    Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $proofAfter } } | Out-Null
} finally {
    # The proof process is terminated below; avoid a blocking socket close on
    # QEMU builds that keep the monitor stream open after screendump.
    $qmp.Client.Client.LingerState = [System.Net.Sockets.LingerOption]::new($false, 0)
    $qmp.Client.Client.Close()
}
if (-not (Test-Path $proofBefore) -or -not (Test-Path $proofAfter)) {
    throw 'QEMU proof did not capture both framebuffer snapshots.'
}
if ((Get-FileHash $proofBefore).Hash -eq (Get-FileHash $proofAfter).Hash) {
    throw 'QEMU keyboard injection did not change the rendered framebuffer.'
}
if (-not $process.HasExited) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
}
$process.Dispose()
Write-Host $result
