<#
Export a commit into a work directory and run a QEMU proof N times, one after another,
each with a fresh disk. Prints one line per run (exit code, duration, failing step),
then per-run first-boot marker counts. Copies each run's logs before the next run
overwrites them.

Example:
  .\qemu-runs.ps1 -Ref origin/codex/issue-36-x -Name pr60 -WorkDir <scratch> -Runs 2 -LockScreen -System -QmpPort 4490
#>
param(
    [Parameter(Mandatory)] [string]$Ref,
    [Parameter(Mandatory)] [string]$Name,
    [Parameter(Mandatory)] [string]$WorkDir,
    [int]$Runs = 2,
    [int]$QmpPort = 4490,
    [switch]$LockScreen,
    [switch]$System,
    [switch]$VirtioGpu,
    [string[]]$ExtraMarkers = @()
)
$repo = (git -C $PSScriptRoot rev-parse --show-toplevel).Trim()
$dir = Join-Path $WorkDir "qr-$Name"
if (-not (Test-Path (Join-Path $dir 'Cargo.toml'))) {
    New-Item -ItemType Directory -Force $dir | Out-Null
    git -C $repo archive --format=tar -o "$dir.tar" $Ref
    tar -xf "$dir.tar" -C $dir
    Remove-Item "$dir.tar"
}
$logs = Join-Path $WorkDir "qr-$Name-logs"
New-Item -ItemType Directory -Force $logs | Out-Null
$markers = @('QEMU proof PASS', 'FATAL', 'Display scene op rejected', 'surface=', 'scene built', 'home surface ready', 'splash ready') + $ExtraMarkers

for ($i = 1; $i -le $Runs; $i++) {
    $disk = $null
    $runArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "$dir\scripts\run.ps1", '-Proof', '-Cpus', '1', '-QmpPort', "$QmpPort")
    if ($LockScreen -or $System) {
        $disk = Join-Path $WorkDir ("qr-$Name-$i-" + (Get-Date -Format 'HHmmss') + '.raw')
        $runArgs += @('-LockScreenProof', '-DiskImage', $disk)
    }
    if ($System) { $runArgs += '-SystemProof' }
    if ($VirtioGpu) { $runArgs += '-VirtioGpu' }
    $start = Get-Date
    # Start-Process keeps cargo's stderr from turning into PowerShell errors.
    $p = Start-Process powershell.exe -ArgumentList $runArgs -WorkingDirectory $dir -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput "$logs\run$i.out" -RedirectStandardError "$logs\run$i.err"
    Copy-Item "$dir\target\qemu-proof-1.log" "$logs\run$i-boot1.log" -ErrorAction SilentlyContinue
    Get-ChildItem "$dir\target\qemu-proof-1-second-*.log" -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -ge $start } |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 |
        ForEach-Object { Copy-Item $_.FullName "$logs\run$i-boot2.log" }
    $failure = Get-Content "$logs\run$i.err" -ErrorAction SilentlyContinue | Select-String "throw '" | Select-Object -First 1
    $reason = if ($p.ExitCode -eq 0) { 'PASS' } else { "$failure".Trim() }
    Write-Output ("run {0}: exit={1} {2:N0}s {3}" -f $i, $p.ExitCode, ((Get-Date) - $start).TotalSeconds, $reason)
    if ($disk) { Remove-Item $disk -ErrorAction SilentlyContinue }
}
Get-ChildItem "$logs\run*-boot*.log" | Sort-Object Name | ForEach-Object {
    $log = $_.FullName
    $counts = foreach ($m in $markers) { '{0}={1}' -f $m, (Select-String -Path $log -SimpleMatch $m).Count }
    '{0}: {1}' -f $_.Name, ($counts -join '; ')
}
Write-Output "screendumps: $dir\target\*.ppm   logs: $logs"
