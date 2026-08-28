param(
    [switch]$Release,
    [switch]$Headless,
    [switch]$Interactive,
    [switch]$Proof,
    [switch]$FetchProof,
    [switch]$NoNetwork,
    [switch]$NetworkProof,
    [switch]$VirtioGpu,
    # Retained as a compatibility alias; networking is enabled by default.
    [switch]$Network,
    [ValidateRange(1, 8)]
    [int]$Cpus = 1,
    [ValidateRange(1, 300)]
    [int]$TimeoutSeconds = 60,
    [string]$DiskImage,
    [ValidateRange(16, 4096)]
    [int]$DiskMiB = 64,
    [ValidateRange(1024, 65535)]
    [int]$QmpPort = 4444,
    [string]$NetworkTracePath
)

$ErrorActionPreference = 'Stop'
if ($FetchProof -and -not $Proof) { throw '-FetchProof requires -Proof.' }
if ($Interactive -and ($Headless -or $Proof)) { throw 'Choose exactly one of -Interactive, -Headless, or -Proof.' }
if ($Network -and $NoNetwork) { throw 'Choose either -Network or -NoNetwork, not both.' }
if ($NoNetwork -and $NetworkProof) { throw 'Choose either -NoNetwork or -NetworkProof, not both.' }
$networkProofEnabled = $Network -or $NetworkProof -or $FetchProof
$networkEnabled = -not $NoNetwork -and (-not $Proof -or $networkProofEnabled)
$interactiveMode = $Interactive -or (-not $Headless -and -not $Proof)
$repoRoot = Split-Path $PSScriptRoot -Parent
$target = Join-Path $repoRoot 'target'
$disk = if ($DiskImage) { [System.IO.Path]::GetFullPath($DiskImage) } else {
    Join-Path $target 'runtime-storage-v5.raw'
}
$profile = if ($Release) { 'release' } else { 'debug' }
$efi = Join-Path $repoRoot "target\x86_64-unknown-uefi\$profile\logos-vnext.efi"
$esp = Join-Path $repoRoot 'target\esp'
$log = Join-Path $repoRoot "target\qemu-proof-$Cpus.log"
$qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
$qemuPath = if ($qemu) { $qemu.Source } else { 'C:\Program Files\qemu\qemu-system-x86_64.exe' }
$ovmf = if ($env:OVMF_CODE) { $env:OVMF_CODE } else { 'C:\Program Files\qemu\share\edk2-x86_64-code.fd' }
if (-not (Test-Path $qemuPath)) { throw 'Install QEMU or add qemu-system-x86_64 to PATH.' }
if (-not (Test-Path $ovmf)) { throw 'Set OVMF_CODE to an OVMF firmware file.' }
New-Item -ItemType Directory -Force $target | Out-Null
if (-not (Test-Path $disk)) {
    $stream = [System.IO.File]::Open($disk, [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write)
    try { $stream.SetLength([int64]$DiskMiB * 1MB) } finally { $stream.Dispose() }
    $marker = New-Object byte[] 4096
    [Text.Encoding]::ASCII.GetBytes('LOGOSBLK').CopyTo($marker, 0)
    $stream = [System.IO.File]::Open($disk, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Write)
    try { $stream.Write($marker, 0, $marker.Length) } finally { $stream.Dispose() }
}

$buildArgs = @('build', '--target', 'x86_64-unknown-uefi')
if ($Proof) {
    $features = @('qemu-proof')
    if ($FetchProof) { $features += 'fetch-proof' }
    $buildArgs += @('--features', ($features -join ','))
}
if ($Release) { $buildArgs += '--release' }
cargo @buildArgs
if ($LASTEXITCODE -ne 0) { throw "Kernel build failed with exit code $LASTEXITCODE." }

& (Join-Path $PSScriptRoot 'build-services.ps1') -Release -Proof:$Proof -FetchProof:$FetchProof
if ($LASTEXITCODE -ne 0) { throw "Service image build failed with exit code $LASTEXITCODE." }

New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\BOOT') | Out-Null
Copy-Item $efi (Join-Path $esp 'EFI\BOOT\BOOTX64.EFI') -Force
New-Item -ItemType Directory -Force (Join-Path $esp 'EFI\LOGOS') | Out-Null
Copy-Item (Join-Path $repoRoot 'build\esp\EFI\LOGOS\*.ELF') (Join-Path $esp 'EFI\LOGOS') -Force
$networkConfig = Join-Path $esp 'EFI\LOGOS\NETWORK.CFG'
if ($networkEnabled) {
    @(
        'profile=static_then_dhcp'
        'address=10.0.2.15'
        'netmask=255.255.255.0'
        'gateway=10.0.2.2'
    ) | Set-Content -LiteralPath $networkConfig -Encoding ascii
} else {
    Remove-Item -LiteralPath $networkConfig -Force -ErrorAction SilentlyContinue
}

$espPath = ((Resolve-Path $esp).Path).Replace('\', '/')
$display = if ($interactiveMode) { 'gtk,zoom-to-fit=on' } else { 'none' }
$qemuArgs = @(
    '-machine', 'q35', '-m', '256M', '-smp', $Cpus,
    '-drive', "if=pflash,format=raw,readonly=on,file=$ovmf",
    '-drive', "format=raw,file=fat:rw:$espPath",
    '-drive', "if=none,id=storage-disk,format=raw,file=$disk,cache=writethrough",
    '-device', 'virtio-blk-pci,drive=storage-disk,disable-legacy=on',
    '-display', $display
)
if ($VirtioGpu) {
    $qemuArgs += @('-device', 'virtio-gpu-pci,id=video0')
} elseif ($Proof) {
    $qemuArgs += @('-device', 'VGA,id=video0')
} else {
    $qemuArgs += @('-vga', 'std')
}
if ($networkEnabled) {
    if ($Proof) {
        $networkPeerPort = $QmpPort + 1
        $qemuArgs += @(
            '-netdev', "socket,id=network0,connect=127.0.0.1:$networkPeerPort",
            '-device', 'virtio-net-pci,netdev=network0,disable-legacy=on'
        )
    } else {
        $qemuArgs += @(
            '-netdev', 'user,id=network0,net=10.0.2.0/24,dhcpstart=10.0.2.15,restrict=off',
            '-device', 'virtio-net-pci,netdev=network0,disable-legacy=on'
        )
    }
}
if ($Proof) {
    Remove-Item $log -Force -ErrorAction SilentlyContinue
    $qemuArgs += @('-no-reboot', '-debugcon', "file:qemu-proof-$Cpus.log", '-global', 'isa-debugcon.iobase=0xe9', '-qmp', "tcp:127.0.0.1:$QmpPort,server=on,wait=off")
} else {
    $qemuArgs += @('-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9')
}

if (-not $Proof) {
    & $qemuPath @qemuArgs
    exit $LASTEXITCODE
}

$psi = [Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $qemuPath
$psi.WorkingDirectory = $target
$psi.Arguments = ($qemuArgs | ForEach-Object {
        if ($_ -match '[\s"]') { '"' + $_.Replace('"', '\"') + '"' } else { $_ }
    }) -join ' '
$psi.UseShellExecute = $false
$process = [Diagnostics.Process]::new()
$process.StartInfo = $psi
[Diagnostics.Process]$networkPeerProcess = $null
if ($networkEnabled -and $Proof) {
    $peerPsi = [Diagnostics.ProcessStartInfo]::new()
    $peerPsi.FileName = (Get-Command powershell.exe -ErrorAction Stop).Source
    $peerPsi.UseShellExecute = $false
    $peerPsi.RedirectStandardOutput = $true
    $peerPsi.RedirectStandardError = $true
    $peerArguments = @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            (Join-Path $PSScriptRoot 'network-peer.ps1'), '-Port', [string]$networkPeerPort
        )
    if ($NetworkTracePath) { $peerArguments += @('-TracePath', [System.IO.Path]::GetFullPath($NetworkTracePath)) }
    foreach ($argument in $peerArguments) {
        [void]$peerPsi.ArgumentList.Add($argument)
    }
    $networkPeerProcess = [Diagnostics.Process]::new()
    $networkPeerProcess.StartInfo = $peerPsi
    [void]$networkPeerProcess.Start()
    Start-Sleep -Milliseconds 100
}
[void]$process.Start()

$deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
$passed = $false
while ([DateTime]::UtcNow -lt $deadline) {
    if (Test-Path $log) {
        $text = Get-Content $log -Raw
        $successMarker = if ($FetchProof) {
            'LogOS vNext: fetch proof PASS'
        } else {
            'LogOS vNext: QEMU proof PASS'
        }
        if ($text -match [regex]::Escape($successMarker)) {
            $passed = $true
            break
        }
        if ($FetchProof -and $text -match 'LogOS vNext: service manager ready') {
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
    if ($process.HasExited) { Write-Host "QEMU exited with code $($process.ExitCode)." }
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    if ($networkPeerProcess -and -not $networkPeerProcess.HasExited) {
        Stop-Process -Id $networkPeerProcess.Id -Force -ErrorAction SilentlyContinue
    }
    if ($networkPeerProcess) {
        $peerError = $networkPeerProcess.StandardError.ReadToEnd()
        if ($peerError) { Write-Host $peerError }
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

function Send-QmpKey {
    param([hashtable]$Qmp, [string]$Key)
    Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{
        execute = 'human-monitor-command'
        arguments = @{ 'command-line' = "sendkey $Key" }
    } | Out-Null
}

function Send-QmpPointerMotion {
    param([hashtable]$Qmp, [int]$X, [int]$Y)
    Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{
        execute = 'input-send-event'
        arguments = @{
            device = 'video0'
            events = @(
                @{ type = 'rel'; data = @{ axis = 'x'; value = $X } }
                @{ type = 'rel'; data = @{ axis = 'y'; value = $Y } }
            )
        }
    } | Out-Null
}

function Send-QmpPointerButton {
    param([hashtable]$Qmp, [bool]$Down)
    Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{
        execute = 'input-send-event'
        arguments = @{
            device = 'video0'
            events = @(
                @{ type = 'btn'; data = @{ button = 'left'; down = $Down } }
            )
        }
    } | Out-Null
}

function Wait-ProofMarker {
    param([string]$Marker, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Test-Path $log) -and (Get-Content $log -Raw) -match [regex]::Escape($Marker)) {
            return $true
        }
        if ($process.HasExited) { return $false }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Get-ProofMarkerCount {
    param([string]$Marker)
    if (-not (Test-Path $log)) { return 0 }
    return ([regex]::Matches((Get-Content $log -Raw), [regex]::Escape($Marker))).Count
}

function Wait-ProofMarkerAfter {
    param([string]$Marker, [int]$MinimumCount, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ((Get-ProofMarkerCount $Marker) -gt $MinimumCount) { return $true }
        if ($process.HasExited) { return $false }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Framebuffer-HasPixels {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    for ($index = 15; $index -lt $bytes.Length; $index++) {
        if ($bytes[$index] -ne 0) { return $true }
    }
    return $false
}

function Framebuffer-HasWhitePixels {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    for ($index = 15; $index -lt $bytes.Length; $index += 3) {
        if ($bytes[$index] -eq 255 -and $bytes[$index + 1] -eq 255 -and $bytes[$index + 2] -eq 255) {
            return $true
        }
    }
    return $false
}

function Framebuffer-HasLockscreenPanel {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $index = 15 + ((300 * 640 + 300) * 3)
    return $bytes[$index] -eq 24 -and $bytes[$index + 1] -eq 37 -and $bytes[$index + 2] -eq 53
}

function Wait-QmpFramebufferStable {
    param([hashtable]$Qmp, [string]$Path, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $previousHash = $null
    $stable = 0
    while ([DateTime]::UtcNow -lt $deadline) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        if ((Framebuffer-HasWhitePixels $Path) -and (Framebuffer-HasLockscreenPanel $Path)) {
            $hash = (Get-FileHash $Path).Hash
            if ($hash -eq $previousHash) {
                $stable++
                if ($stable -ge 2) { return $true }
            } else {
                $previousHash = $hash
                $stable = 0
            }
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Send-QmpKeys {
    param([hashtable]$Qmp, [string[]]$Keys)
    foreach ($key in $Keys) {
        Send-QmpKey $Qmp $key
        Start-Sleep -Milliseconds 25
    }
}

function Send-QmpText {
    param([hashtable]$Qmp, [string]$Text)
    foreach ($character in $Text.ToCharArray()) {
        $key = switch ([int][char]$character) {
            0x0a { 'ret'; break }
            0x20 { 'spc'; break }
            0x22 { '3'; break }       # AZERTY: unshifted 3 is double quote.
            0x28 { '5'; break }       # AZERTY: unshifted 5 is left parenthesis.
            0x29 { 'minus'; break }   # AZERTY: unshifted - is right parenthesis.
            0x2c { 'm'; break }       # AZERTY: unshifted m key is comma.
            0x2e { 'shift-comma'; break } # AZERTY: shifted ; key is period.
            0x2f { 'shift-dot'; break }
            0x6d { ';'; break }       # AZERTY: QMP ; is the physical M key.
            0x3a { 'dot'; break }     # AZERTY: unshifted . key is colon.
            0x30..0x39 { "$character"; break } # AZERTY: digits are unshifted.
            0x61 { 'q'; break }
            0x71 { 'a'; break }
            0x77 { 'z'; break }
            0x7a { 'w'; break }
            default { [char]$character; break }
        }
        Send-QmpKey $Qmp ([string]$key)
        Start-Sleep -Milliseconds 25
    }
}

$proofBefore = Join-Path $repoRoot "target\qemu-proof-before-$Cpus.ppm"
$proofAfter = Join-Path $repoRoot "target\qemu-proof-after-$Cpus.ppm"
$pointerBefore = Join-Path $repoRoot "target\qemu-proof-pointer-before-$Cpus.ppm"
$pointerAfter = Join-Path $repoRoot "target\qemu-proof-pointer-after-$Cpus.ppm"
Remove-Item $proofBefore, $proofAfter, $pointerBefore, $pointerAfter -Force -ErrorAction SilentlyContinue
$qmp = $null
try {
    $qmp = Connect-Qmp $QmpPort
    if ($FetchProof) {
        Start-Sleep -Seconds 2
        Send-QmpText $qmp "net.fetch(`"http://10.0.2.2:8080/readme`",`"/readme`")`n"
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = (Join-Path $repoRoot 'target\fetch-after-input.ppm') } } | Out-Null
        if (-not (Wait-ProofMarker 'LogOS vNext: Flow fetch complete' $TimeoutSeconds)) {
            throw 'Flow fetch did not complete.'
        }
        Send-QmpText $qmp "await fs.open(`"/readme`").read()`n"
        if (-not (Wait-ProofMarker 'LogOS vNext: fetch contents verified' $TimeoutSeconds)) {
            throw 'Fetched contents were not verified.'
        }
        Send-QmpText $qmp "net.fetch(`"http://10.0.2.2:8080/cancel`",`"/cancel`")`n"
        if (-not (Wait-ProofMarker 'LogOS vNext: Flow fetch started' $TimeoutSeconds)) {
            throw 'Cancellation fetch did not start.'
        }
        Send-QmpKey $qmp 'ctrl-c'
        if (-not (Wait-ProofMarker 'LogOS vNext: Flow fetch cancelled' $TimeoutSeconds)) {
            throw 'Fetch cancellation was not observed.'
        }
        Add-Content -LiteralPath $log -Value 'LogOS vNext: fetch proof PASS'
        Write-Host 'Fetch persistence proof PASS'
        return
    }
    $atriumAdmissionCount = Get-ProofMarkerCount 'LogOS vNext: Atrium and LockScreen tasks admitted'
    if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Atrium and LockScreen tasks admitted' $atriumAdmissionCount $TimeoutSeconds)) {
        throw 'Atrium/LockScreen restart admission was not observed.'
    }
    if (-not (Wait-ProofMarker 'LogOS vNext: Atrium IPC topology ready' $TimeoutSeconds)) {
        throw 'Atrium IPC topology was not admitted.'
    }
    if (-not (Wait-ProofMarker 'LogOS vNext: Atrium and LockScreen tasks admitted' $TimeoutSeconds)) {
        throw 'Atrium/LockScreen task startup was not admitted.'
    }
    if ($interactiveMode) {
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $proofBefore } } | Out-Null
    }
    foreach ($key in @('n', 'e', 't', 'dot', 's', 't', 'a', 't', 'u', 's', 'ret')) {
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{
            execute = 'human-monitor-command'
            arguments = @{ 'command-line' = "sendkey $key" }
        } | Out-Null
    }
    Start-Sleep -Milliseconds 500
    if ($interactiveMode) {
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $proofAfter } } | Out-Null
        if (-not (Test-Path $proofBefore) -or -not (Test-Path $proofAfter)) {
            throw 'QEMU proof did not capture both framebuffer snapshots.'
        }
        if ((Get-FileHash $proofBefore).Hash -eq (Get-FileHash $proofAfter).Hash) {
            throw 'QEMU keyboard injection did not change the rendered framebuffer.'
        }
    }
    $resultAfterInput = if (Test-Path $log) { Get-Content $log -Raw } else { '' }
    if ($resultAfterInput -notmatch 'LogOS vNext: keyboard event wake') {
        throw 'QEMU keyboard input did not wake a blocked Input service.'
    }
    # Ensure the known LockScreen surface is live, then exercise PS/2 relative
    # motion and left-button capture/release through QMP.
    Send-QmpKey $qmp 'ctrl-3'
    Start-Sleep -Seconds 2
    $pointerWakeCount = Get-ProofMarkerCount 'LogOS vNext: pointer event wake'
    if (-not (Wait-QmpFramebufferStable $qmp $pointerBefore $TimeoutSeconds)) {
        throw 'QEMU pointer proof did not observe a rendered framebuffer.'
    }
    # PS/2 reports positive Y upward; the decoder converts it to screen-down.
    Send-QmpPointerMotion $qmp 40 -20
    Send-QmpPointerButton $qmp $true
    Send-QmpPointerButton $qmp $false
    if (-not (Wait-ProofMarkerAfter 'LogOS vNext: pointer event wake' $pointerWakeCount $TimeoutSeconds)) {
        throw 'QEMU pointer input did not wake a blocked Input service.'
    }
    if (-not (Wait-QmpFramebufferStable $qmp $pointerAfter $TimeoutSeconds)) {
        throw 'QEMU pointer proof did not settle on a rendered framebuffer.'
    }
    if (-not (Test-Path $pointerBefore) -or -not (Test-Path $pointerAfter)) {
        throw 'QEMU pointer proof did not capture both framebuffer snapshots.'
    }
    if (-not (Framebuffer-HasPixels $pointerAfter)) {
        throw 'QEMU pointer proof lost the rendered framebuffer after input.'
    }
    Write-Host $result
} finally {
    # Stop QEMU before closing the monitor. Some Windows QEMU builds keep the
    # monitor stream open after screendump and otherwise leave this runner stuck.
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
    if ($networkPeerProcess -and -not $networkPeerProcess.HasExited) {
        Stop-Process -Id $networkPeerProcess.Id -Force -ErrorAction SilentlyContinue
    }
    if ($qmp) {
        $qmp.Client.Client.LingerState = [System.Net.Sockets.LingerOption]::new($false, 0)
        $qmp.Client.Client.Close()
    }
    $process.Dispose()
}
