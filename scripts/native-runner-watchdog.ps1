param(
    [Parameter(Mandatory=$true)][int]$ControllerPid,
    [Parameter(Mandatory=$true)][string]$ControllerStartUtc,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ControllerCommandSha256,
    [Parameter(Mandatory=$true)][Guid]$InstanceId,
    [Parameter(Mandatory=$true)][ValidateRange(10,180)][int]$TimeoutMinutes
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1') -ErrorAction Stop
. (Join-Path $PSScriptRoot 'native-runner-receipt.ps1')
. (Join-Path $PSScriptRoot 'native-runner-owner.ps1')
$clock=[Diagnostics.Stopwatch]::StartNew()
$runRoot=Join-Path (Split-Path -Parent $PSScriptRoot) "target\verification\native-runners\$InstanceId"
$owner=$null
try {
    $owner=Open-NativeControllerOwner -ControllerPid $ControllerPid -StartedUtc $ControllerStartUtc -CommandSha256 $ControllerCommandSha256
    while ($true) {
        try {
            $receipt=Read-NativeRunnerReceipt -Path "$runRoot\host-run.json"
            if ($receipt.instance -cne $InstanceId.ToString() -or $receipt.repository -cne 'ropbet-radbyt/sorotte' -or $receipt.runner_name -cne "sorotte-sandbox-$InstanceId") { throw 'Watchdog receipt identity mismatch' }
            if ($receipt.sandbox_stopped -isnot [bool] -or $receipt.runner_removed -isnot [bool]) { throw 'Invalid watchdog cleanup flags' }
            # Older receipts could only reach successful cleanup after token
            # removal; new receipts explicitly retain a failed removal attempt.
            $tokensRemoved=$true
            if ($receipt.PSObject.Properties.Name -contains 'tokens_removed') {
                if ($receipt.tokens_removed -isnot [bool]) { throw 'Invalid watchdog token cleanup flag' }
                $tokensRemoved=$receipt.tokens_removed
            }
            if ($receipt.sandbox_stopped -and $receipt.runner_removed -and $tokensRemoved) { exit 0 }
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
        $ownerAlive=$null -ne $owner -and -not $owner.HasExited
        if (-not $ownerAlive -or $clock.Elapsed.TotalMinutes -gt ($TimeoutMinutes+2)) {
            if ($ownerAlive) {
                Stop-NativeControllerOwner -Owner $owner -StartedUtc $ControllerStartUtc -CommandSha256 $ControllerCommandSha256
                Write-NativeRunnerReceipt -Path "$runRoot\watchdog-observation.json" -Value @{
                    schema_version=1; instance=$InstanceId.ToString(); status='controller-deadline-stopped'
                    observed_at_utc=[DateTime]::UtcNow.ToString('o'); controller_pid=$ControllerPid
                }
            }
            & (Join-Path $PSScriptRoot 'native-runner-sandbox.ps1') -CleanupOnly -InstanceId $InstanceId
            exit $LASTEXITCODE
        }
        Start-Sleep -Seconds 2
    }
} catch {
    Write-NativeRunnerReceipt -Path "$runRoot\watchdog-observation.json" -Value @{
        schema_version=1; instance=$InstanceId.ToString(); status='watchdog-failed'
        observed_at_utc=[DateTime]::UtcNow.ToString('o'); error=$_.Exception.Message
    }
    throw
} finally {
    if ($null -ne $owner) { $owner.Dispose() }
}
