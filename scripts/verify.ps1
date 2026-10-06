param(
    [switch]$Release,
    [switch]$Proof
)

$ErrorActionPreference = 'Stop'
if ($Proof) {
    $failed = @()
    foreach ($cpus in 1, 2, 8) {
        $runParams = @{ Proof = $true; Cpus = $cpus; TimeoutSeconds = 60; Release = $Release }
        $global:LASTEXITCODE = 0
        try {
            & (Join-Path $PSScriptRoot 'run.ps1') @runParams
            if ($LASTEXITCODE -ne 0) { throw "run.ps1 exited with code $LASTEXITCODE" }
        } catch {
            Write-Host "FAIL: -Cpus $cpus : $_"
            $failed += $cpus
        }
    }
    if ($failed.Count -gt 0) { throw "Proof failed for CPU counts: $($failed -join ', ')" }
    exit 0
}

$checkParams = @{ Stage = 'all'; Release = $Release }
& (Join-Path $PSScriptRoot 'check.ps1') @checkParams
