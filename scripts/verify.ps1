param(
    [switch]$Release,
    [switch]$Proof
)

$ErrorActionPreference = 'Stop'
if ($Proof) {
    foreach ($cpus in 1, 2, 8) {
        $runParams = @{ Proof = $true; Cpus = $cpus; TimeoutSeconds = 60; Release = $Release }
        & (Join-Path $PSScriptRoot 'run.ps1') @runParams
    }
    exit 0
}

$checkParams = @{ Stage = 'all'; Release = $Release }
& (Join-Path $PSScriptRoot 'check.ps1') @checkParams
