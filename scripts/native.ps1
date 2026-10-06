# Dot-source this file. Windows PowerShell 5.1 ignores native exit codes under
# $ErrorActionPreference = 'Stop', so wrap each native command in Invoke-Native.
function Invoke-Native {
    param([Parameter(Mandatory)][scriptblock]$Command)
    $global:LASTEXITCODE = 0
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "Native command failed with exit code ${LASTEXITCODE}: $($Command.ToString().Trim())"
    }
}
