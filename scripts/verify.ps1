param([int]$TimeoutSeconds = 15)

$repoRoot = Split-Path $PSScriptRoot -Parent
$log = Join-Path $env:TEMP "logos-qemu-$PID.log"
$errorLog = Join-Path $env:TEMP "logos-qemu-$PID.err.log"
$markers = @(
    'LogOS: check scheduler passed',
    'LogOS: check ipc passed',
    'LogOS: check ipc cancel passed',
    'LogOS: check service task passed',
    'LogOS: check virtio passed',
    'LogOS: check driver recovery passed',
    'LogOS: check service reclamation passed',
    'LogOS: check service replacement passed',
    'LogOS: check supervisor manifest passed',
    'LogOS: check service capability passed',
    'LogOS: check service protocol passed',
    'LogOS: check service diagnostics passed',
    'LogOS: check service health passed',
    'LogOS: check service lifecycle passed',
    'LogOS: check console mode passed',
    'LogOS: console mode normal',
    'LogOS: normal terminal active',
    'LogOS: check command registry passed',
    'LogOS: check input normalization passed',
    'LogOS: check terminal editing passed',
    'LogOS: check terminal navigation passed',
    'LogOS: check terminal layout passed',
    'LogOS: check terminal scrollback passed',
    'LogOS: check terminal history passed',
    'LogOS: check terminal selection passed',
    'LogOS: check terminal caret passed',
    'LogOS: check text font passed',
    'LogOS: startup self check passed'
)
$run = Join-Path $PSScriptRoot 'run.ps1'
$process = Start-Process powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $run, '-Headless') -RedirectStandardOutput $log -RedirectStandardError $errorLog -WindowStyle Hidden -PassThru
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)

try {
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path $log) -and ($markers | Where-Object { -not (Select-String -Path $log -Pattern $_ -Quiet) }).Count -eq 0) {
            Write-Host "Verified: $($markers -join ', ')"
            exit 0
        }
        Start-Sleep -Milliseconds 200
    }
    Get-Content $log -ErrorAction SilentlyContinue
    throw "Timed out waiting for QEMU health markers"
} finally {
    Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $process.Id -and $_.Name -eq 'qemu-system-x86_64.exe' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    Remove-Item $log -ErrorAction SilentlyContinue
    Remove-Item $errorLog -ErrorAction SilentlyContinue
}
