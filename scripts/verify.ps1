param([ValidateSet('main', 'pr', 'nightly', 'weekly')][string]$Suite = 'main')

Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo run -p logos-test -- suite $Suite
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
