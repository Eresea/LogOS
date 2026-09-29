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
# SDL's relative grab recenters the host pointer after capture, so motion does
# not depend on where the user clicked inside the QEMU window.
$display = if ($interactiveMode) { 'sdl' } else { 'none' }
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
    $atriumAdmissionCount = 0 # The proof log was cleared above.
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
    param([hashtable]$Qmp, [int]$X, [int]$Y, [int]$DelayMilliseconds = 150)
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
    if ($DelayMilliseconds -gt 0) { Start-Sleep -Milliseconds $DelayMilliseconds }
}

function Send-QmpPointerWalk {
    param([hashtable]$Qmp, [int]$DeltaY)
    while ($DeltaY -ne 0) {
        $step = [Math]::Sign($DeltaY) * [Math]::Min([Math]::Abs($DeltaY), 2)
        Send-QmpPointerMotion -Qmp $Qmp -X 0 -Y $step -DelayMilliseconds 20
        $DeltaY -= $step
    }
}

# Two-axis version of Send-QmpPointerWalk: the PS/2 decoder reads each
# packet's delta as a signed byte (services/input/src/lib.rs PointerDecoder,
# `self.packet[1] as i8`), so a single large motion (a would-be "clamp to
# (0, 0)" or "jump straight to the target" shortcut) does not travel nearly
# as far as requested -- every motion must walk in small steps, same as
# Send-QmpPointerWalk already does for Y alone.
function Send-QmpPointerWalk2 {
    param([hashtable]$Qmp, [int]$DeltaX, [int]$DeltaY)
    while ($DeltaX -ne 0 -or $DeltaY -ne 0) {
        $stepX = [Math]::Sign($DeltaX) * [Math]::Min([Math]::Abs($DeltaX), 2)
        $stepY = [Math]::Sign($DeltaY) * [Math]::Min([Math]::Abs($DeltaY), 2)
        Send-QmpPointerMotion -Qmp $Qmp -X $stepX -Y $stepY -DelayMilliseconds 15
        $DeltaX -= $stepX
        $DeltaY -= $stepY
    }
}

# At each 2-unit step the PS/2 decoder's acceleration is fixed at 256
# (services/input/src/lib.rs MouseAcceleration::gain: "magnitude <= 2" always
# uses 256, regardless of the mouse acceleration setting), so a raw delta of
# 2 becomes exactly `2 * 224 * 256 / 65536` = 1.75 screen pixels on average
# (the 65536 = POINTER_SCALE remainder carries the .75 to the next step):
# over any multiple of 4 steps (8 raw units), 7 pixels of actual movement.
# Requesting raw = pixels * 8/7 (rounded to the nearest multiple of 8, so
# the remainder cycle divides evenly and this is exact, not asymptotic)
# reaches the intended pixel offset.
function Send-QmpPointerWalkPixels {
    param([hashtable]$Qmp, [int]$PixelsX, [int]$PixelsY)
    $rawX = [Math]::Round(($PixelsX * 8.0 / 7.0) / 8.0) * 8
    $rawY = [Math]::Round(($PixelsY * 8.0 / 7.0) / 8.0) * 8
    Send-QmpPointerWalk2 $Qmp $rawX $rawY
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

function Send-QmpWheel {
    # One notch per call: QMP wheel buttons are press/release pairs (#90).
    param([hashtable]$Qmp, [ValidateSet('wheel-up', 'wheel-down')][string]$Direction)
    foreach ($down in $true, $false) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{
            execute = 'input-send-event'
            arguments = @{
                device = 'video0'
                events = @(@{ type = 'btn'; data = @{ button = $Direction; down = $down } })
            }
        } | Out-Null
    }
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
    # H1: Home's centred search-list panel was replaced by a header and an
    # icon tile grid (services/atrium/src/lib.rs HOME_GRID_* bounds), shown
    # by default (no popover). Tile 1 (Files) is never focused by default, so
    # its icon card reliably shows the unfocused control fill color.
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    if ($layout.Width -le 500 -or $layout.Height -le 700) {
        return $false
    }
    # Below the Home content, this desktop pixel differs from LockScreen fill.
    $desktop = $layout.Offset + ((700 * $layout.Width + 200) * 3)
    # x=30 is inside the 60-pixel rail, away from its controls.
    $rail = $layout.Offset + ((400 * $layout.Width + 30) * 3)
    # x=70 is just outside the rail and should show the desktop background.
    $besideRail = $layout.Offset + ((400 * $layout.Width + 70) * 3)
    # Inside tile index 1's (Files) icon card, offset from its corner so the
    # centered icon glyph itself is not sampled: column_left=360, icon
    # x=360+32=392, y=240; sample point (392+8, 240+8) = (400, 248).
    $tile = $layout.Offset + ((248 * $layout.Width + 400) * 3)
    return $bytes[$desktop] -eq 16 -and $bytes[$desktop + 1] -eq 24 -and $bytes[$desktop + 2] -eq 32 -and
        $bytes[$rail] -eq 24 -and $bytes[$rail + 1] -eq 37 -and $bytes[$rail + 2] -eq 53 -and
        $bytes[$besideRail] -eq 16 -and $bytes[$besideRail + 1] -eq 24 -and $bytes[$besideRail + 2] -eq 32 -and
        $bytes[$tile] -eq 38 -and $bytes[$tile + 1] -eq 53 -and $bytes[$tile + 2] -eq 72
}

function Framebuffer-HasHomeSelectedCard {
    # Tile index 0 (Calculator) holds keyboard focus by default; its icon
    # card should show the focus fill color, sampled off-center so the
    # centered icon glyph is not hit. Column_left=160, icon x=160+32=192,
    # y=240; sample point (192+8, 240+8) = (200, 248).
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    $x = 200
    $y = 248
    $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
    return $bytes[$index] -eq 62 -and $bytes[$index + 1] -eq 114 -and $bytes[$index + 2] -eq 217
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

function Wait-QmpSystemFramebuffer {
    param([hashtable]$Qmp, [string]$Path, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        if ((Framebuffer-HasSystemStatusBar $Path) -and (Framebuffer-HasSystemRows $Path)) {
            return $true
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

function Framebuffer-PixelIsWhite {
    # S5 (#82): `UiSceneTheme::LIGHT.panel` and `SYSTEM_THEME_LIGHT.panel`
    # (ui-graphics/src/lib.rs, services/images/src/system.rs) are both pure
    # white, so any known dark-theme `panel` sample point (already used by
    # Framebuffer-HasHomePanel/HasSystemStatusBar/SettingsCategorySelected)
    # doubles as a one-pixel light-theme check.
    param([string]$Path, [int]$X, [int]$Y)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    if ($X -ge $layout.Width -or $Y -ge $layout.Height) { return $false }
    $index = $layout.Offset + (($Y * $layout.Width + $X) * 3)
    return $bytes[$index] -eq 255 -and $bytes[$index + 1] -eq 255 -and $bytes[$index + 2] -eq 255
}

function Wait-QmpPixelIsWhite {
    # Requires the match twice in a row, like Wait-QmpSettingsCategory,
    # so a screendump caught mid-repaint doesn't pass on a stale frame.
    param([hashtable]$Qmp, [string]$Path, [int]$X, [int]$Y, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $previousHash = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        if (Framebuffer-PixelIsWhite $Path $X $Y) {
            $hash = (Get-FileHash $Path).Hash
            if ($hash -eq $previousHash) { return $true }
            $previousHash = $hash
        } else {
            $previousHash = $null
        }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Framebuffer-PixelMatches {
    param([string]$Path, [int]$X, [int]$Y, [int]$R, [int]$G, [int]$B)
    if (-not (Test-Path $Path)) { return $false }
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    if ($X -ge $layout.Width -or $Y -ge $layout.Height) { return $false }
    $index = $layout.Offset + (($Y * $layout.Width + $X) * 3)
    return $bytes[$index] -eq $R -and $bytes[$index + 1] -eq $G -and $bytes[$index + 2] -eq $B
}

function Wait-QmpPixelMatches {
    param([hashtable]$Qmp, [string]$Path, [int]$X, [int]$Y, [int]$R, [int]$G, [int]$B, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $previousHash = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        if (Framebuffer-PixelMatches $Path $X $Y $R $G $B) {
            $hash = (Get-FileHash $Path).Hash
            if ($hash -eq $previousHash) { return $true }
            $previousHash = $hash
        } else {
            $previousHash = $null
        }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Framebuffer-SettingsCategorySelected {
    param([string]$Path, [int]$Selected)
    if (-not (Test-Path $Path)) { return $false }
    $match = [regex]::Matches((Get-Content $log -Raw), 'app=Settings scene published surface=\d+/\d+ bounds=(-?\d+),(-?\d+),(\d+),(\d+)')
    if ($match.Count -eq 0) { return $false }
    $groups = $match[$match.Count - 1].Groups
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    # Category rows start at surface (32, 164) and step 48 px (44 + 4 gap).
    # Sample each row's left padding, clear of its icon and label: the
    # selected row carries the focus fill (0x3e72d9), idle rows show the
    # navigation pane (0x182535) through their transparent fill.
    for ($row = 0; $row -lt 4; $row++) {
        $x = [int]$groups[1].Value + 36
        $y = [int]$groups[2].Value + 164 + 48 * $row + 22
        $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
        $accent = $bytes[$index] -eq 62 -and $bytes[$index + 1] -eq 114 -and $bytes[$index + 2] -eq 217
        $pane = $bytes[$index] -eq 24 -and $bytes[$index + 1] -eq 37 -and $bytes[$index + 2] -eq 53
        if (($row -eq $Selected -and -not $accent) -or ($row -ne $Selected -and -not $pane)) {
            return $false
        }
    }
    # The selected row's label must stay painted above its fill.
    $top = [int]$groups[2].Value + 164 + 48 * $Selected
    for ($y = $top + 8; $y -lt $top + 36; $y++) {
        for ($x = [int]$groups[1].Value + 80; $x -lt [int]$groups[1].Value + 200; $x++) {
            $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
            if ($bytes[$index] -gt 200 -and $bytes[$index + 1] -gt 200 -and $bytes[$index + 2] -gt 200) {
                return $true
            }
        }
    }
    return $false
}

function Wait-QmpSettingsCategory {
    param([hashtable]$Qmp, [string]$Path, [int]$Selected, [int]$TimeoutSeconds)
    # Require the matching frame twice in a row so the whole page update,
    # not only the category row, has landed before the screendump is kept.
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $previousHash = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        if (Framebuffer-SettingsCategorySelected $Path $Selected) {
            $hash = (Get-FileHash $Path).Hash
            if ($hash -eq $previousHash) { return $true }
            $previousHash = $hash
        } else {
            $previousHash = $null
        }
        Start-Sleep -Milliseconds 250
    }
    return $false
}

function Framebuffer-HasTerminalGlyphs {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return $false }
    # Terminal bounds come from Atrium's scene-published marker (#74); the
    # content starts below the 32 px chrome (TERMINAL_CHROME_HEIGHT) plus the
    # 28 px tab strip (TERMINAL_TAB_BAR_HEIGHT, #76) so this never samples
    # tab-chip pixels as if they were terminal glyphs. A glyph is a bright
    # pixel. The bottom 40 px is excluded so the always-on white FPS counter
    # (bottom-right) can never false-pass this check.
    $match = [regex]::Matches((Get-Content $log -Raw), 'app=Terminal scene published surface=\d+/\d+ bounds=(-?\d+),(-?\d+),(\d+),(\d+)')
    if ($match.Count -eq 0) { return $false }
    $groups = $match[$match.Count - 1].Groups
    $left = [int]$groups[1].Value; $top = [int]$groups[2].Value + 60
    $right = $left + [int]$groups[3].Value
    $bytes = [IO.File]::ReadAllBytes($Path)
    $layout = Get-PpmLayout $bytes
    $bottom = [Math]::Min([int]$groups[2].Value + [int]$groups[4].Value, $layout.Height - 40)
    for ($y = [Math]::Max($top, 0); $y -lt $bottom -and $y -lt $layout.Height; $y++) {
        for ($x = [Math]::Max($left, 0); $x -lt $right -and $x -lt $layout.Width; $x++) {
            $index = $layout.Offset + (($y * $layout.Width + $x) * 3)
            if ($bytes[$index] -ge 160 -and $bytes[$index + 1] -ge 160 -and $bytes[$index + 2] -ge 160) {
                return $true
            }
        }
    }
    return $false
}

function Wait-QmpTerminalFramebuffer {
    param([hashtable]$Qmp, [string]$Path, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        if (Framebuffer-HasTerminalGlyphs $Path) {
            return $true
        }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

# The Terminal surface's own content rect (from the Atrium marker, below the
# chrome and tab strip, bottom 40 px excluded so the ever-changing FPS
# counter never counts) as a flat byte array, for content-only comparisons
# that ignore the rest of the frame.
function Get-TerminalContentBytes {
    param([byte[]]$Bytes)
    $match = [regex]::Matches((Get-Content $log -Raw), 'app=Terminal scene published surface=\d+/\d+ bounds=(-?\d+),(-?\d+),(\d+),(\d+)')
    $groups = $match[$match.Count - 1].Groups
    $layout = Get-PpmLayout $Bytes
    $left = [Math]::Max([int]$groups[1].Value, 0)
    $top = [Math]::Max([int]$groups[2].Value + 60, 0)
    $right = [Math]::Min($left + [int]$groups[3].Value, $layout.Width)
    $bottom = [Math]::Min([int]$groups[2].Value + [int]$groups[4].Value, $layout.Height - 40)
    $rowLength = ($right - $left) * 3
    $content = [byte[]]::new(($bottom - $top) * $rowLength)
    for ($y = $top; $y -lt $bottom; $y++) {
        $rowStart = $layout.Offset + ($y * $layout.Width + $left) * 3
        [Array]::Copy($Bytes, $rowStart, $content, ($y - $top) * $rowLength, $rowLength)
    }
    return $content
}

function Test-BytesEqual {
    param([byte[]]$A, [byte[]]$B)
    if ($A.Length -ne $B.Length) { return $false }
    for ($i = 0; $i -lt $A.Length; $i++) {
        if ($A[$i] -ne $B[$i]) { return $false }
    }
    return $true
}

# Screendumps repeatedly until the Terminal's own content rect stops
# changing between two consecutive dumps (session/Flow output has finished
# draining), or gives up at the deadline and returns the last dump taken.
function Wait-QmpTerminalContentSettled {
    param([hashtable]$Qmp, [string]$Path, [int]$TimeoutSeconds)
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
    $previous = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($Path))
    $stableStreak = 0
    while ([DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 500
        Invoke-QmpCommand $Qmp.Writer $Qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $Path } } | Out-Null
        $current = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($Path))
        if (Test-BytesEqual $previous $current) {
            # Require 3 consecutive identical dumps (not just 2): a slow,
            # bursty drain can otherwise leave a multi-hundred-ms gap between
            # bursts that looks "stable" for one interval but isn't done yet.
            $stableStreak++
            if ($stableStreak -ge 3) {
                return $true
            }
        } else {
            $stableStreak = 0
        }
        $previous = $current
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
        } elseif (-not (Framebuffer-HasNativeCursor $pointerAfter 688 376)) {
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
        # LoginLayout places the claim fields at y=168, 224, 280, and 336.
        # Walk from the decoder's y=400 center to the username at y=188, then
        # advance through the remaining rows. Steps of at most 2 use the
        # decoder's unaccelerated 224/256 base gain; 243 then 64 raw counts
        # move -212 then 55/56 pixels, keeping each click within its field.
        Send-QmpPointerWalk $qmp -243
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' $pointerTargetCount $TimeoutSeconds)) {
            throw 'Register username click was not delivered.'
        }
        Send-QmpText $qmp 'admin'
        Send-QmpPointerWalk $qmp 64
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' ($pointerTargetCount + 1) $TimeoutSeconds)) {
            throw 'Register password click was not delivered.'
        }
        Send-QmpText $qmp 'password'
        Send-QmpPointerWalk $qmp 64
        Send-QmpPointerButton $qmp $true
        Send-QmpPointerButton $qmp $false
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen pointer target accepted' ($pointerTargetCount + 2) $TimeoutSeconds)) {
            throw 'Register confirmation click was not delivered.'
        }
        Send-QmpText $qmp 'password'
        Send-QmpPointerWalk $qmp 64
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
        $usernameRedrawMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen input redraw submitted'
        Send-QmpText $qmp 'admin'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen username value changed' $usernameValueMarker $TimeoutSeconds)) {
            throw 'Keyboard input did not change the LockScreen username value.'
        }
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
        $passwordRedrawMarker = Get-ProofMarkerCount 'LogOS vNext: LockScreen input redraw submitted'
        Send-QmpText $qmp 'password'
        if (-not (Wait-ProofMarkerAfter 'LogOS vNext: LockScreen password value changed' $passwordValueMarker $TimeoutSeconds)) {
            throw 'Keyboard input did not change the LockScreen password value.'
        }
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
        $homeFrame = Join-Path $repoRoot "target\qemu-home-$PID.ppm"
        $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
        do {
            Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'screendump'; arguments = @{ filename = $homeFrame } } | Out-Null
            $homeReady = (Framebuffer-HasHomePanel $homeFrame) -and (Framebuffer-HasHomeSelectedCard $homeFrame)
            if (-not $homeReady) { Start-Sleep -Milliseconds 250 }
        } until ($homeReady -or [DateTime]::UtcNow -ge $deadline)
        if (-not $homeReady) {
            throw 'Post-login home surface did not publish its popover pixels.'
        }
        if ($SystemProof) {
            # #78: Settings opens on its category list. Settings is the last
            # Home tile (index 4); Down moves the selection to Mouse, then
            # Appearance. Escape closes Settings and Left x4 returns grid
            # focus to Calculator so the System step starts from its tile.
            $settingsSceneMarker = Get-ProofMarkerCount 'LogOS vNext: Atrium app=Settings scene published'
            1..4 | ForEach-Object {
                Send-QmpKey $qmp 'right'
                Start-Sleep -Milliseconds 100
            }
            Send-QmpKey $qmp 'ret'
            if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Atrium app=Settings scene published' $settingsSceneMarker $TimeoutSeconds)) {
                throw 'Settings did not publish its scene after activation.'
            }
            $settingsFrame = Join-Path $repoRoot "target\qemu-settings-$PID.ppm"
            if (-not (Wait-QmpSettingsCategory $qmp $settingsFrame 0 $TimeoutSeconds)) {
                throw 'Settings did not open with the Keyboard category selected.'
            }
            Send-QmpKey $qmp 'down'
            $settingsMouseFrame = Join-Path $repoRoot "target\qemu-settings-mouse-$PID.ppm"
            if (-not (Wait-QmpSettingsCategory $qmp $settingsMouseFrame 1 $TimeoutSeconds)) {
                throw 'Down did not move the Settings selection to the Mouse category.'
            }
            # #79: the Appearance page (accent swatches, FPS and motion rows).
            Send-QmpKey $qmp 'down'
            $settingsAppearanceFrame = Join-Path $repoRoot "target\qemu-settings-appearance-$PID.ppm"
            if (-not (Wait-QmpSettingsCategory $qmp $settingsAppearanceFrame 2 $TimeoutSeconds)) {
                throw 'Down did not move the Settings selection to the Appearance category.'
            }
            # #80: the About page (version and service list, read-only).
            Send-QmpKey $qmp 'down'
            $settingsAboutFrame = Join-Path $repoRoot "target\qemu-settings-about-$PID.ppm"
            if (-not (Wait-QmpSettingsCategory $qmp $settingsAboutFrame 3 $TimeoutSeconds)) {
                throw 'Down did not move the Settings selection to the About category.'
            }
            Send-QmpKey $qmp 'esc'
            Start-Sleep -Milliseconds 300
            1..4 | ForEach-Object {
                Send-QmpKey $qmp 'left'
                Start-Sleep -Milliseconds 100
            }

            $systemSceneMarker = Get-ProofMarkerCount 'LogOS vNext: System scene built'
            # H1: Home's default view is the header + icon tile grid (arrow
            # keys move focus across tiles, Enter opens); System sits at tile
            # index 3 (Calculator, Files, Terminal, System, Settings), so
            # Right x3 from the default Calculator focus reaches it.
            1..3 | ForEach-Object {
                Send-QmpKey $qmp 'right'
                Start-Sleep -Milliseconds 100
            }
            Send-QmpKey $qmp 'ret'
            Send-QmpKey $qmp 'ctrl-4'
            if (-not (Wait-ProofMarkerAfter 'LogOS vNext: System scene built' $systemSceneMarker $TimeoutSeconds)) {
                throw 'System service did not build its scene after activation.'
            }
            $systemFrame = Join-Path $repoRoot "target\qemu-system-$PID.ppm"
            if (-not (Wait-QmpSystemFramebuffer $qmp $systemFrame $TimeoutSeconds)) {
                throw 'System surface did not publish its status bar and service rows.'
            }

            # #74: Terminal content renders through the retained text-grid
            # node. Open Terminal, type a command, and assert the reply
            # reaches Display as `GuiTextGridRow`s that paint real glyph
            # pixels inside the Terminal surface's own content bounds.
            $terminalSceneMarker = Get-ProofMarkerCount 'LogOS vNext: Atrium app=Terminal scene published'
            $terminalRowMarker = Get-ProofMarkerCount 'LogOS vNext: Display text grid row applied'
            Send-QmpKey $qmp 'ctrl-3'
            if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Atrium app=Terminal scene published' $terminalSceneMarker $TimeoutSeconds)) {
                throw 'Terminal did not publish its scene after activation.'
            }
            Send-QmpText $qmp 'help'
            Send-QmpKey $qmp 'ret'
            if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Display text grid row applied' $terminalRowMarker $TimeoutSeconds)) {
                throw 'Terminal output did not reach Display as text-grid rows.'
            }
            $terminalFrame = Join-Path $repoRoot "target\qemu-terminal-$PID.ppm"
            if (-not (Wait-QmpTerminalFramebuffer $qmp $terminalFrame $TimeoutSeconds)) {
                throw 'Terminal surface did not publish glyph pixels inside its content bounds.'
            }

            # #75: bounded scrollback. Overflow the visible rows (each `help`
            # round trip prints a few lines; enough repeats push well past a
            # full-height pane's ~47 rows and into scrollback), screendump
            # the live bottom view, then scroll up with Shift+PageUp and
            # screendump again. The "Display text grid row applied" marker
            # is a one-shot latch (logged only for the very first row ever
            # applied, see `TEXT_GRID_ROW_LOGGED` in
            # services/images/src/display.rs), so it can't confirm this
            # second batch landed; comparing the two framebuffers' bytes for
            # a difference proves scrolling actually changed the visible
            # content instead.
            1..100 | ForEach-Object {
                Send-QmpText $qmp 'help'
                Send-QmpKey $qmp 'ret'
                # Let each round trip (echo + Flow reply + new prompt, then
                # the row-by-row drain to Display) land before typing the
                # next one; sending faster than that pipeline drains has
                # been observed to drop or garble characters. 100 repeats at
                # this pace comfortably clears a full-height pane's ~47
                # rows even when a good fraction of individual repeats are
                # lost to that pacing.
                Start-Sleep -Milliseconds 400
            }
            # Wait for the whole backlog of typed commands to actually
            # finish draining (session/Flow replies, then the row-by-row
            # relay to Display) before touching scroll: `feed()` calls
            # `show_live_view()` on every new byte of session output, so
            # scrolling up while output is still trickling in snaps straight
            # back to live and would make the comparison below a false
            # negative. Poll until the content rect itself stops changing
            # rather than guessing a fixed delay.
            $terminalBottomFrame = Join-Path $repoRoot "target\qemu-terminal-bottom-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalBottomFrame $TimeoutSeconds)) {
                throw 'Terminal overflow output never stopped changing (drain did not settle).'
            }
            Send-QmpKey $qmp 'shift-pgup'
            $terminalScrolledFrame = Join-Path $repoRoot "target\qemu-terminal-scrolled-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalScrolledFrame $TimeoutSeconds)) {
                throw 'Terminal did not settle after scrolling up.'
            }
            $bottomContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalBottomFrame))
            $scrolledContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalScrolledFrame))
            if (Test-BytesEqual $bottomContent $scrolledContent) {
                throw 'Scrolling up did not change the Terminal framebuffer (scrollback not visible).'
            }

            # #90: mouse-wheel scrollback. Back to the live view first, then
            # wheel up (must scroll, so the frame differs from live) and
            # wheel down (must return to exactly the live frame).
            Send-QmpKey $qmp 'shift-pgdn'
            $terminalWheelFrame = Join-Path $repoRoot "target\qemu-terminal-wheel-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalBottomFrame $TimeoutSeconds)) {
                throw 'Terminal did not settle on the live view before the wheel proof.'
            }
            $wheelLiveContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalBottomFrame))
            1..3 | ForEach-Object { Send-QmpWheel $qmp 'wheel-up' }
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalWheelFrame $TimeoutSeconds)) {
                throw 'Terminal did not settle after wheel-up.'
            }
            $wheelUpContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalWheelFrame))
            if (Test-BytesEqual $wheelLiveContent $wheelUpContent) {
                throw 'Mouse wheel up did not scroll the Terminal scrollback.'
            }
            1..3 | ForEach-Object { Send-QmpWheel $qmp 'wheel-down' }
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalBottomFrame $TimeoutSeconds)) {
                throw 'Terminal did not settle after wheel-down.'
            }
            $wheelDownContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalBottomFrame))
            if (-not (Test-BytesEqual $wheelLiveContent $wheelDownContent)) {
                throw 'Mouse wheel down did not return the Terminal to the live view.'
            }
            Write-Host "Mouse wheel scrolled the Terminal scrollback (frame: $terminalWheelFrame)"

            # #76: T3 sessions. Snap back to the live view (scrolled away
            # above), open a second tab with Ctrl+Shift+T, type a marker
            # only this tab has seen, and screendump it. Switch back to the
            # first tab with Ctrl+Tab and screendump that too: the two
            # frames must differ (distinct per-tab content), and the first
            # tab's content must match what it showed before the second tab
            # ever existed (switching away and back never touched it).
            Send-QmpKey $qmp 'shift-pgdn'
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalBottomFrame $TimeoutSeconds)) {
                throw 'Terminal did not settle back on the live view before opening a second tab.'
            }
            $tabOneLiveContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalBottomFrame))
            Send-QmpKey $qmp 'ctrl-shift-t'
            # Lowercase: Send-QmpText's AZERTY key-name table only remaps a
            # handful of known characters and does not shift for uppercase,
            # so a capitalized marker would silently type as nothing.
            Send-QmpText $qmp 'echo("tabtwo")'
            Send-QmpKey $qmp 'ret'
            $terminalTabTwoFrame = Join-Path $repoRoot "target\qemu-terminal-tab2-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalTabTwoFrame $TimeoutSeconds)) {
                throw 'Second Terminal tab did not settle after typing its own command.'
            }
            $tabTwoContent = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalTabTwoFrame))
            if (Test-BytesEqual $tabOneLiveContent $tabTwoContent) {
                throw 'Second Terminal tab shows the same content as the first (tabs are not independent).'
            }
            Send-QmpKey $qmp 'ctrl-tab'
            $terminalTabOneFrame = Join-Path $repoRoot "target\qemu-terminal-tab1-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalTabOneFrame $TimeoutSeconds)) {
                throw 'First Terminal tab did not settle after switching back to it.'
            }
            $tabOneContentAfterSwitch = Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalTabOneFrame))
            if (-not (Test-BytesEqual $tabOneLiveContent $tabOneContentAfterSwitch)) {
                throw 'Switching tabs and back changed the first tab''s own content.'
            }
            if (Test-BytesEqual $tabOneContentAfterSwitch $tabTwoContent) {
                throw 'The first tab shows the second tab''s content after switching back.'
            }

            # #98 (T3c): per-session Flow state. Submit a command in tab
            # one, then — without waiting for its reply — immediately
            # switch to tab two and type there. Before #98 this raced a
            # single shared Session/Flow instance: tab two's keystrokes
            # could echo into tab one's still-in-flight exchange. Tab
            # two's own echo must land cleanly and distinctly, and
            # switching back to tab one must still show its own reply
            # once it arrives, proving the two sessions' line-editor and
            # Flow variable state never cross.
            Send-QmpText $qmp 'help'
            Send-QmpKey $qmp 'ret'
            Send-QmpKey $qmp 'ctrl-tab'
            Send-QmpText $qmp 'echo("tabtwoinflight")'
            Send-QmpKey $qmp 'ret'
            $terminalTabTwoInFlightFrame = Join-Path $repoRoot "target\qemu-terminal-tab2-inflight-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalTabTwoInFlightFrame $TimeoutSeconds)) {
                throw 'Second tab did not settle after typing while the first tab had a command in flight.'
            }
            $tabTwoInFlightContent =
                Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalTabTwoInFlightFrame))
            if (Test-BytesEqual $tabOneContentAfterSwitch $tabTwoInFlightContent) {
                throw 'Typing in the second tab while the first had a command in flight leaked into the first.'
            }
            Send-QmpKey $qmp 'ctrl-tab'
            $terminalTabOneAfterFlightFrame = Join-Path $repoRoot "target\qemu-terminal-tab1-after-flight-$PID.ppm"
            if (-not (Wait-QmpTerminalContentSettled $qmp $terminalTabOneAfterFlightFrame $TimeoutSeconds)) {
                throw 'First tab did not settle on its own reply after its command was submitted concurrently.'
            }
            $tabOneAfterFlightContent =
                Get-TerminalContentBytes ([IO.File]::ReadAllBytes($terminalTabOneAfterFlightFrame))
            if (Test-BytesEqual $tabOneAfterFlightContent $tabTwoInFlightContent) {
                throw 'The first tab shows the second tab''s content after its own command completed.'
            }

            # S5 (#82): the light theme. Everything above ran in the dark
            # theme (default) and its pixel checks are dark-coded, so the
            # switch happens last, after them, not before. Both Terminal
            # and System are still open (System was never closed after its
            # own check, and opening Terminal with Ctrl+3 does not close
            # it): Escape closes only the focused surface, so send it twice
            # to close Terminal then System and return to the home grid,
            # then reset grid focus to Calculator's tile the same way the
            # Settings-close step above does, before reopening Settings.
            Send-QmpKey $qmp 'esc'
            Start-Sleep -Milliseconds 300
            Send-QmpKey $qmp 'esc'
            Start-Sleep -Milliseconds 300
            1..4 | ForEach-Object {
                Send-QmpKey $qmp 'left'
                Start-Sleep -Milliseconds 100
            }
            1..4 | ForEach-Object {
                Send-QmpKey $qmp 'right'
                Start-Sleep -Milliseconds 100
            }
            $settingsReopenMarker = Get-ProofMarkerCount 'LogOS vNext: Atrium app=Settings scene published'
            Send-QmpKey $qmp 'ret'
            if (-not (Wait-ProofMarkerAfter 'LogOS vNext: Atrium app=Settings scene published' $settingsReopenMarker $TimeoutSeconds)) {
                throw 'Settings did not publish its scene after reopening for the light-theme check.'
            }
            # Settings tiles inside the desktop area (it excludes the 60px
            # shell rail), so read its actual surface origin from the log
            # rather than assuming (0, 0), the same way
            # Framebuffer-SettingsCategorySelected does.
            $settingsMatch = [regex]::Matches((Get-Content $log -Raw), 'app=Settings scene published surface=\d+/\d+ bounds=(-?\d+),(-?\d+),(\d+),(\d+)')
            if ($settingsMatch.Count -eq 0) {
                throw 'Could not find the Settings surface bounds in the proof log.'
            }
            $settingsGroups = $settingsMatch[$settingsMatch.Count - 1].Groups
            $settingsOriginX = [int]$settingsGroups[1].Value
            $settingsOriginY = [int]$settingsGroups[2].Value
            # Settings keeps its category across close/reopen (it's Atrium
            # state, not reset on launch): the very first open above walked
            # Down to About (index 3, the last category), which would still
            # be selected now. Click the Appearance row directly
            # (services/atrium/src/lib.rs settings_category_bounds(2))
            # instead of walking Up/Down from an assumed start -- it selects
            # Appearance regardless of where the category was left. Walk far
            # up-left first to pin the cursor at (0, 0) regardless of where
            # it was left (services/input/src/lib.rs clamps to the screen
            # bounds; -3000/-2000 raw is comfortably past the screen size
            # even from the far corner, see Send-QmpPointerWalkPixels for
            # the raw-to-pixel ratio), so every walk below is relative to a
            # known origin.
            Send-QmpPointerWalk2 $qmp -3000 -2000
            Send-QmpPointerWalkPixels $qmp ($settingsOriginX + 136) ($settingsOriginY + 282)
            Start-Sleep -Milliseconds 150
            Send-QmpPointerButton $qmp $true
            Send-QmpPointerButton $qmp $false
            $settingsAppearanceDarkFrame = Join-Path $repoRoot "target\qemu-settings-appearance-predark-$PID.ppm"
            if (-not (Wait-QmpSettingsCategory $qmp $settingsAppearanceDarkFrame 2 $TimeoutSeconds)) {
                throw 'Settings did not reopen on the Appearance category before the light-theme click.'
            }
            # Click the Light theme row (services/atrium/src/lib.rs
            # settings_light_theme_row_bounds; it reuses the select
            # trigger's node slots, see settings_scene.rs build_appearance,
            # instead of its own three nodes -- MAX_GUI_NODES has no spare
            # node for a third toggle). Re-clamp to (0, 0) first since the
            # category click above already moved the cursor.
            Send-QmpPointerWalk2 $qmp -3000 -2000
            Send-QmpPointerWalkPixels $qmp ($settingsOriginX + 630) ($settingsOriginY + 378)
            Start-Sleep -Milliseconds 150
            Send-QmpPointerButton $qmp $true
            Send-QmpPointerButton $qmp $false
            # Row 0 (Keyboard) is unselected and idle, so its pane pixel
            # reads the theme's `panel` colour directly -- white in
            # `UiSceneTheme::LIGHT`, unlike the accent- or focus-filled
            # pixels `Framebuffer-SettingsCategorySelected` samples.
            $settingsAppearanceLightFrame = Join-Path $repoRoot "target\qemu-settings-appearance-light-$PID.ppm"
            if (-not (Wait-QmpPixelIsWhite $qmp $settingsAppearanceLightFrame ($settingsOriginX + 36) ($settingsOriginY + 186) $TimeoutSeconds)) {
                throw 'Settings did not switch to the light theme after the toggle click.'
            }
            # S4 (#81, ADR-0091): the light-theme toggle above also queues a
            # settings save to User, which only acks `Ok` after its own
            # durable Storage round trip (persist_catalog). Wait for that
            # before quitting below, so the persisted-boot check after the
            # reboot is testing durability, not a lucky timing window.
            if (-not (Wait-ProofMarker 'LogOS vNext: Atrium settings saved' $TimeoutSeconds)) {
                throw 'Atrium did not durably save settings after the light-theme toggle.'
            }

            Send-QmpKey $qmp 'esc'
            $homeLightFrame = Join-Path $repoRoot "target\qemu-home-light-$PID.ppm"
            # The shell rail ((30, 400), Framebuffer-HasHomePanel's dark
            # sample point) is Atrium chrome painted regardless of which
            # surface has focus, so it already reads the new theme's white
            # the instant the toggle is clicked -- it can't tell "Home is
            # showing" from "Settings is still open". The default-focused
            # Calculator tile's accent fill (same point
            # Framebuffer-HasHomeSelectedCard samples) is Home-only content,
            # not shared with Settings' own layout at that point, so it
            # only turns this colour once Home is actually back on screen.
            # Settings (tile index 4, not Calculator's index 0) still holds
            # grid focus at this point -- closing it doesn't reset focus to
            # Calculator, only the explicit Left x4 walk below does -- so
            # (1000, 248) (HOME_GRID_LEFT=160 + 4*(160+40)=960, +32 to the
            # icon, +8,8 off-centre, services/atrium/src/lib.rs
            # home_grid_item_bounds) samples the Settings tile, matching
            # Framebuffer-HasHomeSelectedCard's tile0 pattern shifted to
            # tile4. 111,156,235 = 0x6f9ceb = UI_ACCENTS_LIGHT[Blue].focus
            # (ui-graphics/src/lib.rs), the default accent.
            if (-not (Wait-QmpPixelMatches $qmp $homeLightFrame 1000 248 111 156 235 $TimeoutSeconds)) {
                throw 'Home did not switch to the light theme after Settings closed.'
            }

            1..4 | ForEach-Object {
                Send-QmpKey $qmp 'left'
                Start-Sleep -Milliseconds 100
            }
            1..3 | ForEach-Object {
                Send-QmpKey $qmp 'right'
                Start-Sleep -Milliseconds 100
            }
            Send-QmpKey $qmp 'ret'
            $systemLightFrame = Join-Path $repoRoot "target\qemu-system-light-$PID.ppm"
            # (100, 20) is System's status bar, sampled by
            # `Framebuffer-HasSystemStatusBar` for the dark theme's `panel`
            # colour; `SYSTEM_THEME_LIGHT.panel` (services/images/src/system.rs)
            # is white too.
            if (-not (Wait-QmpPixelIsWhite $qmp $systemLightFrame 100 20 $TimeoutSeconds)) {
                throw 'System did not switch to the light theme.'
            }
        }

        $homeMarker = Get-ProofMarkerCount 'LogOS vNext: Atrium home surface ready'
        # Linux QEMU may close the QMP socket before replying to quit;
        # the WaitForExit below is the real check.
        try { Invoke-QmpCommand $qmp.Writer $qmp.Reader @{ execute = 'quit' } | Out-Null } catch { }
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
        # Only -SystemProof toggles the light theme, so only it can expect the reload.
        # S4 (#81, ADR-0091): settings persistence proof. The first boot
        # above toggled the light theme (durably saved -- see the
        # 'Atrium settings saved' wait before quitting) before this reboot
        # on the same disk image. Atrium logs which value it loaded back
        # from User before its first render, so this is a log-marker check
        # of the persisted setting, not a pixel match.
        if ($SystemProof -and -not (Wait-ProofMarker 'LogOS vNext: Atrium settings loaded light_theme=1' $TimeoutSeconds)) {
            throw 'Settings did not persist across reboot: light theme was not loaded back as on.'
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
