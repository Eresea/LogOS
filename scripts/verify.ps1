param([switch]$Release)

$ErrorActionPreference = 'Stop'
$args = @('-Stage', 'uefi')
if ($Release) { $args += '-Release' }
& (Join-Path $PSScriptRoot 'check.ps1') @args
