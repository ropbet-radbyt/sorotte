param(
    [Parameter(Mandatory=$true)][string]$PortableToolsRoot,
    [string]$OutputDirectory=(Join-Path (Split-Path -Parent $PSScriptRoot) ('target\native-runner-inputs\'+[Guid]::NewGuid().ToString()))
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Staging is reversible and does not enable Sandbox or register a runner.
# A fresh output seals the complete selected portable compiler/tool inventory.
& python (Join-Path $PSScriptRoot 'native_runner_bundle.py') prepare `
    --tools-root (Resolve-Path -LiteralPath $PortableToolsRoot).Path --bundle ([IO.Path]::GetFullPath($OutputDirectory))
if ($LASTEXITCODE -ne 0) { throw 'Native input preparation failed; no runner was registered' }
Write-Host "Reviewed native input bundle: $OutputDirectory"
