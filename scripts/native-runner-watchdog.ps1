param(
    [Parameter(Mandatory=$true)][int]$ControllerPid,
    [Parameter(Mandatory=$true)][string]$ControllerStartUtc,
    [Parameter(Mandatory=$true)][Guid]$InstanceId,
    [Parameter(Mandatory=$true)][ValidateRange(10,180)][int]$TimeoutMinutes
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'native-runner-receipt.ps1')
$clock=[Diagnostics.Stopwatch]::StartNew()
$runRoot=Join-Path (Split-Path -Parent $PSScriptRoot) "target\verification\native-runners\$InstanceId"
while ($true) {
    try {
        $receipt=Read-NativeRunnerReceipt -Path "$runRoot\host-run.json"
        if ($receipt.instance -cne $InstanceId.ToString() -or $receipt.repository -cne 'ropbet-radbyt/sorotte' -or $receipt.runner_name -cne "sorotte-sandbox-$InstanceId") { throw 'Watchdog receipt identity mismatch' }
        if ($receipt.sandbox_stopped -isnot [bool] -or $receipt.runner_removed -isnot [bool]) { throw 'Invalid watchdog cleanup flags' }
        if ($receipt.sandbox_stopped -and $receipt.runner_removed) { exit 0 }
    } catch {
        # An unavailable observation must not stop ownership/deadline checks.
        # Recovery still validates the retained receipt before touching anything.
        try {
            Write-NativeRunnerReceipt -Path "$runRoot\watchdog-observation.json" -Value @{
                schema_version=1; instance=$InstanceId.ToString(); status='receipt-read-unavailable'
                observed_at_utc=[DateTime]::UtcNow.ToString('o')
            }
        } catch { Write-Warning 'Watchdog could not retain its unavailable receipt observation' }
    }
    $owner=Get-Process -Id $ControllerPid -ErrorAction SilentlyContinue
    $ownerAlive=$null -ne $owner -and $owner.StartTime.ToUniversalTime().ToString('o') -ceq $ControllerStartUtc
    if (-not $ownerAlive -or $clock.Elapsed.TotalMinutes -gt ($TimeoutMinutes+2)) {
        & (Join-Path $PSScriptRoot 'native-runner-sandbox.ps1') -CleanupOnly -InstanceId $InstanceId
        exit $LASTEXITCODE
    }
    Start-Sleep -Seconds 2
}
