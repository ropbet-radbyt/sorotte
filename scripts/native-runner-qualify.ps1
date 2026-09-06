[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9_./-]*$')][string]$ReviewedRef,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{40}$')][string]$SourceSha,
    [Parameter(Mandatory=$true)][string]$BundleDirectory,
    [ValidateSet('none','96','144','192')][string]$NativeDpi='none',
    [ValidateRange(10,180)][int]$TimeoutMinutes=90
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1') -ErrorAction Stop
. (Join-Path $PSScriptRoot 'gui-native-smoke-process.ps1')
. (Join-Path $PSScriptRoot 'native-runner-receipt.ps1')
. (Join-Path $PSScriptRoot 'native-runner-cleanup.ps1')
$repo='ropbet-radbyt/sorotte'
$repoRoot=Split-Path -Parent $PSScriptRoot
function Api([string]$Path) {
    $raw=& gh.exe api $Path
    if ($LASTEXITCODE -ne 0) { throw 'Native qualification GitHub lookup failed' }
    ($raw -join "`n") | ConvertFrom-Json
}
$refCommit=Api "repos/$repo/commits/$([Uri]::EscapeDataString($ReviewedRef))"
if ($refCommit.sha -cne $SourceSha) { throw 'Reviewed ref does not resolve to the exact authorized source' }
$actor=Api 'user'
$permission=Api "repos/$repo/collaborators/$($actor.login)/permission"
if ($permission.permission -notin @('write','maintain','admin')) { throw 'Native dispatch requires repository maintainer authority' }
$runsPath="repos/$repo/actions/workflows/gui-native-interactive.yml/runs?head_sha=$SourceSha&per_page=100"
$observed=@((Api $runsPath).workflow_runs)
$before=@($observed | ForEach-Object { [long]$_.id })
$active=@($observed | Where-Object { $_.status -ne 'completed' -and $_.head_sha -ceq $SourceSha -and $_.head_repository.full_name -ceq $repo -and ($_.event -eq 'workflow_dispatch' -or ($_.event -in @('push','schedule') -and $_.head_branch -ceq 'main')) })
if ($active.Count -gt 1) { throw 'Multiple matching native runs are active; use the exact run/job controller command' }
$attemptRoot=Join-Path $repoRoot ('target\verification\native-requests\'+[Guid]::NewGuid().ToString())
$null=New-Item -ItemType Directory -Path $attemptRoot
$receipt=[ordered]@{schema_version=1;kind='sorotte-native-request';source_sha=$SourceSha;reviewed_ref=$ReviewedRef;actor=$actor.login;status='dispatching';run_id=$null;run_attempt=$null;job_id=$null;instance=$null;started=[DateTime]::UtcNow.ToString('o');error=$null;cancellation='not-needed'}
function Save-Receipt { $receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $attemptRoot 'request.json') -Encoding utf8 }
Save-Receipt
try {
    $runId=$null
    if ($active.Count -eq 1) {
        if ($NativeDpi -ne 'none') { throw 'A native run is already active; finish it before requesting a separate actual-DPI profile' }
        $runId=$active[0].id
    }
    else {
        & gh.exe workflow run gui-native-interactive.yml --repo $repo --ref $ReviewedRef -f "source_sha=$SourceSha" -f "native_dpi=$NativeDpi"
        if ($LASTEXITCODE -ne 0) { throw 'Exact reviewed native workflow dispatch failed' }
    }
    $queued=[Diagnostics.Stopwatch]::StartNew()
    $nativeJob=$null
    while ($queued.Elapsed.TotalMinutes -lt 10) {
        if ($null -eq $runId) {
            $new=@((Api $runsPath).workflow_runs | Where-Object { [long]$_.id -notin $before -and $_.event -eq 'workflow_dispatch' -and $_.actor.login -ceq $actor.login })
            if ($new.Count -gt 1) { throw 'Concurrent matching native dispatches are ambiguous; use the exact run/job controller command' }
            if ($new.Count -eq 1) { $runId=$new[0].id }
        }
        if ($null -ne $runId) {
            $run=Api "repos/$repo/actions/runs/$runId"
            $receipt.run_id=$run.id
            $receipt.run_attempt=$run.run_attempt
            $receipt.status='awaiting-native-job'
            Save-Receipt
            if ($run.status -eq 'completed') { throw "Native workflow completed before provisioning: $($run.conclusion)" }
            $jobs=@((Api "repos/$repo/actions/runs/$($run.id)/attempts/$($run.run_attempt)/jobs?per_page=100").jobs | Where-Object { 'sorotte-native-interactive' -in $_.labels })
            if ($jobs.Count -gt 1) { throw 'Native dispatch has ambiguous privileged jobs' }
            if ($jobs.Count -eq 1 -and $jobs[0].status -eq 'queued') { $nativeJob=$jobs[0]; break }
        }
        Start-Sleep -Seconds 3
    }
    if ($null -eq $nativeJob) { throw 'Hosted applicability did not queue the exact native job within ten minutes' }
    foreach ($approver in @($run.actor.login,$run.triggering_actor.login)) {
        if ($approver -notmatch '^[A-Za-z0-9][A-Za-z0-9-]{0,38}$') { throw 'Native producer requires an identifiable maintainer approval' }
        $authority=Api "repos/$repo/collaborators/$approver/permission"
        if ($authority.user.login -ine $approver -or $authority.permission -notin @('write','maintain','admin')) { throw 'Native producer approver lacks current repository write authority' }
    }
    $receipt.job_id=$nativeJob.id
    $receipt.instance=[Guid]::NewGuid().ToString()
    $receipt.status='provisioning'
    Save-Receipt
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'native-runner-sandbox.ps1') `
        -BundleDirectory $BundleDirectory -SourceSha $SourceSha -RunId $receipt.run_id `
        -RunAttempt $receipt.run_attempt -JobId $receipt.job_id -TimeoutMinutes $TimeoutMinutes -InstanceId $receipt.instance
    if ($LASTEXITCODE -ne 0) { throw 'Native qualification or isolated cleanup failed; retain this request and its controller attempt' }
    $receipt.status='passed'
}
catch {
    $primaryFailure=$_
    $receipt.status='failed'
    $receipt.error=$_.Exception.Message
    Save-Receipt
    if ($null -ne $runId) {
        try {
            $cleanup=New-NativeCleanupContext (Join-Path $attemptRoot ('cancel-'+[Guid]::NewGuid().ToString())) 30000 (Join-Path $attemptRoot 'request.json')
            $receipt.cancellation=Stop-NativeQualificationRequest $cleanup $receipt
        }
        catch { $receipt.cancellation='unconfirmed'; $receipt['cancellation_error']=$_.Exception.Message }
    }
    throw $primaryFailure
}
finally { Save-Receipt }
