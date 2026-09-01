param(
    [switch]$Release,
    [switch]$Debug,
    [switch]$Headless,
    [switch]$Interactive,
    [switch]$Proof,
    [switch]$LockScreenProof,
    [switch]$SystemProof,
    [switch]$FetchProof,
    [switch]$NoNetwork,
    [switch]$NetworkProof,
    [switch]$VirtioGpu,
    [switch]$InputTrace,
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
if ($Release -and $Debug) { throw 'Choose either -Release or -Debug, not both.' }
if ($FetchProof -and -not $Proof) { throw '-FetchProof requires -Proof.' }
if ($LockScreenProof -and -not $Proof) { throw '-LockScreenProof requires -Proof.' }
if ($SystemProof -and -not $LockScreenProof) { throw '-SystemProof requires -LockScreenProof.' }
if ($LockScreenProof -and -not $DiskImage) {
    throw '-LockScreenProof requires a fresh -DiskImage path.'
}
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
if ($LockScreenProof -and (Test-Path $disk)) {
    throw "-LockScreenProof requires a new disk image; already exists: $disk"
}
$releaseBuild = $Release -or -not $Debug
$profile = if ($releaseBuild) { 'release' } else { 'debug' }
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
if ($Proof -or $InputTrace) {
    $features = @(
        if ($Proof) { 'qemu-proof' } else { 'input-debug' }
    )
    if ($LockScreenProof) { $features += 'lockscreen-proof' }
    if ($FetchProof) { $features += 'fetch-proof' }
    $buildArgs += @('--features', ($features -join ','))
}
if ($releaseBuild) { $buildArgs += '--release' }
cargo @buildArgs
if ($LASTEXITCODE -ne 0) { throw "Kernel build failed with exit code $LASTEXITCODE." }

& (Join-Path $PSScriptRoot 'build-services.ps1') -Release -Proof:$Proof -InputDebug:$InputTrace -LockScreenProof:$LockScreenProof -FetchProof:$FetchProof
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
$inputTracePath = Join-Path $target "qemu-input-$PID.trace"
$inputDebugLogPath = Join-Path $target "qemu-input-$PID.log"
$display = if ($interactiveMode) { 'gtk,zoom-to-fit=off' } else { 'none' }
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
} else {
    # Keep the host window and relative pointer coordinates in the same
    # geometry as the GOP mode selected by the kernel.
    $qemuArgs += @('-device', 'VGA,id=video0,xres=1280,yres=800')
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
    if ($InputTrace) {
        Remove-Item -LiteralPath $inputDebugLogPath -Force -ErrorAction SilentlyContinue
        $qemuArgs += @('-debugcon', "file:$inputDebugLogPath", '-global', 'isa-debugcon.iobase=0xe9', '-qmp', "tcp:127.0.0.1:$QmpPort,server=on,wait=off")
    } else {
        $qemuArgs += @('-debugcon', 'stdio', '-global', 'isa-debugcon.iobase=0xe9')
    }
}
if ($InputTrace) {
    Remove-Item -LiteralPath $inputTracePath -Force -ErrorAction SilentlyContinue
    $qemuArgs += @('-trace', "enable=*keyboard*,file=$inputTracePath")
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
    $peerPsi.Arguments = ($peerArguments | ForEach-Object {
            if ($_ -match '[\s"]') { '"' + $_.Replace('"', '\"') + '"' } else { $_ }
        }) -join ' '
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
        } elseif ($LockScreenProof) {
            'LogOS vNext: LockScreen claim mode ready'
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
                @{ type = 'rel'; data = @{ axis = 'x'; value = $X } },
                @{ type = 'rel'; data = @{ axis = 'y'; value = $Y } }
            )
        }
    } | Out-Null
    Start-Sleep -Milliseconds 150
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
    Start-Sleep -Milliseconds 100
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
    $layout = Get-PpmLayout $bytes
    for ($index = $layout.Offset; $index -lt $bytes.Length; $index++) {
        if ($bytes[$index] -ne 0) { return $true }
    }
    return $false
}

function Framebuffer-HasWhitePixels {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    for ($index = $layout.Offset; $index -lt $bytes.Length; $index += 3) {
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
    $layout = Get-PpmLayout $bytes
    # Sample just beside the centered cursor so the proof checks the panel,
    # not the cursor's adaptive bright/dark core.
    $centerX = [int]($layout.Width / 2) - 20
    $centerY = [int]($layout.Height / 2)
    $index = $layout.Offset + (($centerY * $layout.Width + $centerX) * 3)
    return $bytes[$index] -eq 24 -and $bytes[$index + 1] -eq 37 -and $bytes[$index + 2] -eq 53
}

function Framebuffer-HasHomePanel {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    # This point is inside the home popover, away from text, the cursor, and
    # the rounded shadow. It must not remain the root background.
    $x = 500
    $y = 140
    if ($x -lt 0 -or $y -lt 0 -or $x -ge $layout.Width -or $y -ge $layout.Height) {
        return $false
    }
    $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
    return $bytes[$index] -eq 24 -and $bytes[$index + 1] -eq 37 -and $bytes[$index + 2] -eq 53
}

function Framebuffer-HasHomeSelectedCard {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    $x = 400
    $y = 320
    $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
    return $bytes[$index] -eq 53 -and $bytes[$index + 1] -eq 107 -and $bytes[$index + 2] -eq 216
}

function Framebuffer-HasSystemStatusBar {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    $x = 100
    $y = 20
    $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
    return $bytes[$index] -eq 24 -and $bytes[$index + 1] -eq 37 -and $bytes[$index + 2] -eq 53
}

function Framebuffer-HasSystemRows {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    for ($y = 76; $y -lt 145 -and $y -lt $layout.Height; $y++) {
        for ($x = 20; $x -lt 250 -and $x -lt $layout.Width; $x++) {
            $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
            if ($bytes[$index] -ne 16 -or $bytes[$index + 1] -ne 24 -or $bytes[$index + 2] -ne 32) {
                return $true
            }
        }
    }
    return $false
}

function Framebuffer-HasNativeCursor {
    param([string]$Path, [int]$X, [int]$Y)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    # The software cursor is an adaptive circular shape with an opaque core.
    $points = @(@(0, 0), @(1, 0), @(-1, 0), @(0, 1), @(0, -1))
    $layout = Get-PpmLayout $bytes
    if ($X -lt 0 -or $Y -lt 0 -or $X -ge $layout.Width -or $Y -ge $layout.Height) { return $false }
    $origin = $layout.Offset + (($Y * $layout.Width + $X) * 3)
    $red = $bytes[$origin]
    $green = $bytes[$origin + 1]
    $blue = $bytes[$origin + 2]
    $isBright = $red -ge 220 -and $green -ge 220 -and $blue -ge 220
    $isDark = $red -le 32 -and $green -le 32 -and $blue -le 32
    if (-not ($isBright -or $isDark)) { return $false }
    foreach ($point in $points) {
        $pointX = $X + $point[0]
        $pointY = $Y + $point[1]
        if ($pointX -lt 0 -or $pointY -lt 0 -or $pointX -ge $layout.Width -or $pointY -ge $layout.Height) { return $false }
        $index = $layout.Offset + ((($pointY * $layout.Width) + $pointX) * 3)
        if ($bytes[$index] -ne $red -or $bytes[$index + 1] -ne $green -or $bytes[$index + 2] -ne $blue) {
            return $false
        }
    }
    return $true
}

function Get-PpmLayout {
    param([byte[]]$Bytes)
    $newlineCount = 0
    $headerEnd = 0
    for ($index = 0; $index -lt [Math]::Min($Bytes.Length, 128); $index++) {
        if ($Bytes[$index] -eq 10) {
            $newlineCount++
            if ($newlineCount -eq 3) {
                $headerEnd = $index + 1
                break
            }
        }
    }
    if ($headerEnd -eq 0) { throw 'Invalid PPM header.' }
    $header = [Text.Encoding]::ASCII.GetString($Bytes, 0, $headerEnd).Trim() -split '\s+'
    if ($header.Count -lt 4 -or $header[0] -ne 'P6') { throw 'Unsupported framebuffer dump.' }
    @{ Offset = $headerEnd; Width = [int]$header[1]; Height = [int]$header[2] }
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
$keyboardBefore = Join-Path $repoRoot "target\qemu-proof-keyboard-before-$Cpus.ppm"
$keyboardAfter = Join-Path $repoRoot "target\qemu-proof-keyboard-after-$Cpus.ppm"
Remove-Item $proofBefore, $proofAfter, $pointerBefore, $pointerAfter, $keyboardBefore, $keyboardAfter -Force -ErrorAction SilentlyContinue
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
    if (-not $LockScreenProof) {
        $atriumAdmissionCount = Get-ProofMarkerCount 'LogOS vNext: Atrium and LockScreen tasks admitted'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Atrium and LockScreen tasks admitted' $atriumAdmissionCount $TimeoutSeconds)) {
            throw 'Atrium/LockScreen restart admission was not observed.'
        }
    }
    if (-not (Wait-ProofMarker 'LogOS vNext: Atrium IPC topology ready' $TimeoutSeconds)) {
        throw 'Atrium IPC topology was not admitted.'
    }
    if (-not (Wait-ProofMarker 'LogOS vNext: Atrium locked route ready' $TimeoutSeconds)) {
        throw 'Boot did not enter the locked route.'
    }
    if (-not (Wait-ProofMarker 'LogOS vNext: Atrium and LockScreen tasks admitted' $TimeoutSeconds)) {
        throw 'Atrium/LockScreen task startup was not admitted.'
    }
    if (-not $LockScreenProof) {
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen cursor surface ready' $TimeoutSeconds)) {
        throw 'LockScreen cursor surface did not become ready.'
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
        # The PS/2 decoder intentionally drops zero-delta packets. Use one
        # bounded pixel of motion to materialize the initial software cursor.
        $cursorBaselineCount = Get-ProofMarkerCount 'LogOS vNext: Display cursor published'
        Send-QmpPointerMotion $qmp 1 0
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: pointer event wake' $pointerWakeCount $TimeoutSeconds)) {
        throw 'QEMU pointer baseline event was not delivered.'
        }
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Display cursor published' $cursorBaselineCount $TimeoutSeconds)) {
        throw 'QEMU pointer baseline cursor was not published.'
        }
        if (-not (Wait-QmpFramebufferStable $qmp $pointerBefore $TimeoutSeconds)) {
        throw 'QEMU pointer proof did not observe a rendered framebuffer.'
        }
        if (-not $VirtioGpu -and -not (Framebuffer-HasNativeCursor $pointerBefore 641 400)) {
        throw 'QEMU pointer proof did not observe the native cursor on LockScreen.'
        }
        if ($VirtioGpu -and -not (Wait-ProofMarker 'LogOS vNext: VirtIO GPU cursor ready' $TimeoutSeconds)) {
        throw 'QEMU VirtIO-GPU proof did not publish the hardware cursor.'
        }
    # QEMU's relative Y axis is converted by the PS/2 device before the
    # decoder applies its screen-down convention.
        Start-Sleep -Seconds 2
        $cursorPublishCount = Get-ProofMarkerCount 'LogOS vNext: Display cursor published'
        Send-QmpPointerMotion $qmp 40 -20
        Start-Sleep -Milliseconds 250
        Send-QmpPointerButton $qmp $true
        Start-Sleep -Milliseconds 100
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: pointer event wake' $pointerWakeCount $TimeoutSeconds)) {
        throw 'QEMU pointer input did not wake a blocked Input service.'
        }
        if (-not $VirtioGpu -and -not (Wait-ProofMarkerAfter 'LogOS vNext: Display cursor published' $cursorPublishCount $TimeoutSeconds)) {
        throw 'QEMU pointer input did not publish the moved software cursor.'
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
        if ($VirtioGpu) {
        if (-not (Wait-ProofMarker 'LogOS vNext: VirtIO GPU cursor moved' $TimeoutSeconds)) {
            throw 'QEMU pointer motion did not move the VirtIO-GPU cursor plane.'
        }
        } elseif (-not (Framebuffer-HasNativeCursor $pointerAfter 681 380)) {
        throw 'QEMU pointer motion did not move the native cursor to the decoded position.'
        }
    }
    if ($LockScreenProof) {
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen surface ready' $TimeoutSeconds)) {
        throw 'LockScreen surface did not become ready.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen splash ready' $TimeoutSeconds)) {
        throw 'LockScreen splash overlay did not become ready.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen cursor surface ready' $TimeoutSeconds)) {
        throw 'LockScreen cursor surface did not become ready.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen claim mode ready' $TimeoutSeconds)) {
        throw 'First-boot Claim mode did not become ready.'
        }
        # Exercise the rendered register controls with the mouse: username,
        # password, confirmation, then the submit target.
        $pointerTargetCount = Get-ProofMarkerCount 'LogOS vNext: LockScreen pointer target accepted'
        # The claim controls are centered in the 1280x800 viewport. Start at
        # the decoder's centered pointer position and walk the rendered rows.
        Send-QmpPointerMotion $qmp 0 -100
        Send-QmpPointerMotion $qmp 0 -100
        Send-QmpPointerMotion $qmp 0 -100
        Send-QmpPointerMotion $qmp 0 -12
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' $pointerTargetCount $TimeoutSeconds)) {
            throw 'Register username click was not delivered.'
        }
        Send-QmpText $qmp 'admin'
        Send-QmpPointerMotion $qmp 0 60
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' ($pointerTargetCount + 1) $TimeoutSeconds)) {
            throw 'Register password click was not delivered.'
        }
        Send-QmpText $qmp 'password'
        Send-QmpPointerMotion $qmp 0 60
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' ($pointerTargetCount + 2) $TimeoutSeconds)) {
            throw 'Register confirmation click was not delivered.'
        }
        Send-QmpText $qmp 'password'
        Send-QmpPointerMotion $qmp 0 60
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' ($pointerTargetCount + 3) $TimeoutSeconds)) {
            throw 'Register submit click was not delivered.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen admin claim PASS' $TimeoutSeconds)) {
            throw 'Admin claim did not complete.'
        }
        if (-not (Wait-QmpFramebufferStable $qmp $keyboardBefore $TimeoutSeconds)) {
            throw 'LockScreen login framebuffer did not stabilize before keyboard input.'
        }
        $loginMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen login PASS'
        $usernameValueMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen username value changed'
        $passwordValueMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen password value changed'
        Send-QmpText $qmp 'admin'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen username value changed' $usernameValueMarker $TimeoutSeconds)) {
            throw 'Keyboard input did not change the LockScreen username value.'
        }
        $usernameRedrawMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen input redraw submitted'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen input redraw submitted' $usernameRedrawMarker $TimeoutSeconds)) {
            throw 'Keyboard username input did not redraw the LockScreen.'
        }
        if (-not (Wait-QmpFramebufferStable $qmp $keyboardAfter $TimeoutSeconds)) {
            throw 'LockScreen login framebuffer did not stabilize after username input.'
        }
        if ((Get-FileHash $keyboardBefore).Hash -eq (Get-FileHash $keyboardAfter).Hash) {
            throw 'Keyboard username input did not change the rendered LockScreen.'
        }
        Send-QmpKey $qmp 'tab'
        Send-QmpText $qmp 'password'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen password value changed' $passwordValueMarker $TimeoutSeconds)) {
            throw 'Keyboard input did not change the LockScreen password value.'
        }
        $passwordRedrawMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen input redraw submitted'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen input redraw submitted' $passwordRedrawMarker $TimeoutSeconds)) {
            throw 'Keyboard password input did not redraw the LockScreen.'
        }
        Send-QmpKey $qmp 'ret'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen login PASS' $loginMarker $TimeoutSeconds)) {
            throw 'Post-claim login did not complete.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: Atrium authenticated' $TimeoutSeconds)) {
            throw 'Atrium did not receive the explicit login session.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: Atrium home surface ready' $TimeoutSeconds)) {
            throw 'Home surface did not become ready after explicit login.'
        }
        Start-Sleep -Seconds 2
        $homeFrame = Join-Path $repoRoot "target\qemu-home-$PID.ppm"
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $homeFrame } } | Out-Null
        if (-not (Framebuffer-HasHomePanel $homeFrame) -or -not (Framebuffer-HasHomeSelectedCard $homeFrame)) {
            throw 'Post-login home surface did not publish its popover pixels.'
        }
        if ($SystemProof) {
            $systemSceneMarker = Get-ProofMarkerCount 'LogOS vNext: System scene built'
            1..3 | ForEach-Object {
                Send-QmpKey $qmp 'down'
                Start-Sleep -Milliseconds 100
            }
            Send-QmpKey $qmp 'ret'
            Send-QmpKey $qmp 'ctrl-4'
            if (-not (Wait-ProofMarkerAfter 'LogOS vNext: System scene built' $systemSceneMarker $TimeoutSeconds)) {
                throw 'System service did not build its scene after activation.'
            }
            Start-Sleep -Seconds 2
            $systemFrame = Join-Path $repoRoot "target\qemu-system-$PID.ppm"
            Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $systemFrame } } | Out-Null
            if (-not (Framebuffer-HasSystemStatusBar $systemFrame) -or -not (Framebuffer-HasSystemRows $systemFrame)) {
                throw 'System surface did not publish its status bar and service rows.'
            }
        }

        $homeMarker = Get-ProofMarkerCount 'LogOS vNext: Atrium home surface ready'
        Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'quit' } | Out-Null
        if (-not $process.WaitForExit(5000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            throw 'First proof QEMU did not exit before the persisted boot.'
        }
        $qmp.Client.Close()
        $qmp = $null
        $secondLogName = "qemu-proof-$Cpus-second-$PID.log"
        $secondLog = Join-Path $target $secondLogName
        Remove-Item -LiteralPath $secondLog -Force -ErrorAction SilentlyContinue
        $qemuArgs = $qemuArgs | ForEach-Object {
            if ($_ -like 'file:qemu-proof-*.log') { "file:$secondLogName" } else { $_ }
        }
        $psi.Arguments = ($qemuArgs | ForEach-Object {
                if ($_ -match '[\s"]') { '"' + $_.Replace('"', '\"') + '"' } else { $_ }
            }) -join ' '
        $process.Dispose()
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $psi
        [void]$process.Start()
        $log = $secondLog
        $homeMarker = Get-ProofMarkerCount 'LogOS vNext: Atrium home surface ready'
        $qmp = Connect-Qmp $QmpPort
        if (-not (Wait-ProofMarker 'LogOS vNext: Atrium IPC topology ready' $TimeoutSeconds)) {
            throw 'Second boot did not reach Atrium.'
        }
        if (-not (Wait-ProofMarker 'LogOS vNext: LockScreen surface ready' $TimeoutSeconds)) {
            throw 'Second boot did not recreate LockScreen.'
        }
        $loginMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen login PASS'
        Send-QmpText $qmp 'admin'
        Send-QmpKey $qmp 'tab'
        Send-QmpText $qmp 'password'
        Send-QmpKey $qmp 'ret'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen login PASS' $loginMarker $TimeoutSeconds)) {
            throw 'Persisted admin login did not complete.'
        }
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Atrium home surface ready' $homeMarker $TimeoutSeconds)) {
            throw 'Home surface did not become ready after persisted login.'
        }
        Write-Host 'LockScreen two-boot proof PASS'
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
