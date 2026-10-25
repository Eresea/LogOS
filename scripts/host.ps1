param(
    [switch]$Release
)

$ErrorActionPreference = 'Stop'
$profileArgs = @()
if ($Release) { $profileArgs += '--release' }

cargo build -p logos-service-images --bins -p logos-host @profileArgs
if ($LASTEXITCODE -ne 0) { throw "Host build failed with exit code $LASTEXITCODE." }

cargo run -p logos-host @profileArgs
if ($LASTEXITCODE -ne 0) { throw "logos-host exited with code $LASTEXITCODE." }
