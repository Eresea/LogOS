param([int]$TimeoutSeconds = 15)

$repoRoot = Split-Path $PSScriptRoot -Parent
$log = Join-Path $env:TEMP "logos-qemu-$PID.log"
$errorLog = Join-Path $env:TEMP "logos-qemu-$PID.err.log"
$marker = 'LogOS: startup self check passed'
$run = Join-Path $PSScriptRoot 'run.ps1'
$process = Start-Process powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $run, '-Headless') -RedirectStandardOutput $log -RedirectStandardError $errorLog -WindowStyle Hidden -PassThru
$deadline = (Get-Date).AddSeconds($TimeoutSeconds)

try {
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path $log) -and (Select-String -Path $log -Pattern $marker -Quiet)) {
            Write-Host "Verified: $marker"
            exit 0
        }
        Start-Sleep -Milliseconds 200
    }
    Get-Content $log -ErrorAction SilentlyContinue
    throw "Timed out waiting for $marker"
} finally {
    Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $process.Id -and $_.Name -eq 'qemu-system-x86_64.exe' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    Remove-Item $log -ErrorAction SilentlyContinue
    Remove-Item $errorLog -ErrorAction SilentlyContinue
}
