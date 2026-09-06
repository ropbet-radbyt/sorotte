param(
    [Parameter(Mandatory=$true)][int]$ControllerPid,
    [Parameter(Mandatory=$true)][string]$ControllerStartUtc,
    [Parameter(Mandatory=$true)][Guid]$InstanceId,
    [Parameter(Mandatory=$true)][ValidateRange(10,180)][int]$TimeoutMinutes
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$clock=[Diagnostics.Stopwatch]::StartNew()
$runRoot=Join-Path (Split-Path -Parent $PSScriptRoot) "target\verification\native-runners\$InstanceId"
while ($true) {
    $receipt=Get-Content -LiteralPath "$runRoot\host-run.json" -Raw | ConvertFrom-Json
    if ($receipt.sandbox_stopped -and $receipt.runner_removed) { exit 0 }
    $owner=Get-Process -Id $ControllerPid -ErrorAction SilentlyContinue
    $ownerAlive=$null -ne $owner -and $owner.StartTime.ToUniversalTime().ToString('o') -ceq $ControllerStartUtc
    if (-not $ownerAlive -or $clock.Elapsed.TotalMinutes -gt ($TimeoutMinutes+2)) {
        & (Join-Path $PSScriptRoot 'native-runner-sandbox.ps1') -CleanupOnly -InstanceId $InstanceId
        exit $LASTEXITCODE
    }
    Start-Sleep -Seconds 2
}
