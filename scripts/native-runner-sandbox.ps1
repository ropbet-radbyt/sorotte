[CmdletBinding(DefaultParameterSetName='Run')]
param(
    [Parameter(Mandatory=$true, ParameterSetName='Run')][string]$BundleDirectory,
    [Parameter(Mandatory=$true, ParameterSetName='Run')][ValidatePattern('^[0-9a-f]{40}$')][string]$SourceSha,
    [Parameter(Mandatory=$true, ParameterSetName='Run')][ValidateRange(1,[long]::MaxValue)][long]$RunId,
    [Parameter(Mandatory=$true, ParameterSetName='Run')][ValidateRange(1,100000)][int]$RunAttempt,
    [Parameter(Mandatory=$true, ParameterSetName='Run')][ValidateRange(1,[long]::MaxValue)][long]$JobId,
    [Parameter(ParameterSetName='Run')][ValidateRange(10,180)][int]$TimeoutMinutes=90,
    [Parameter(Mandatory=$true, ParameterSetName='Recover')][switch]$CleanupOnly,
    [Parameter(Mandatory=$true, ParameterSetName='Recover')]
    [Parameter(ParameterSetName='Run')][Guid]$InstanceId=[Guid]::NewGuid()
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
$runBase=[IO.Path]::GetFullPath((Join-Path $repoRoot 'target\verification\native-runners'))
$runRoot=[IO.Path]::GetFullPath((Join-Path $runBase $InstanceId.ToString()))
if (-not $runRoot.StartsWith($runBase+'\',[StringComparison]::OrdinalIgnoreCase) -or $InstanceId -eq [Guid]::Empty) { throw 'Invalid isolated instance path' }
$repo='ropbet-radbyt/sorotte'
$runnerName="sorotte-sandbox-$InstanceId"
$tokenPath=Join-Path $runRoot 'output\registration-token.json'
$receiptPath=Join-Path $runRoot 'host-run.json'
# Recovery must reach each independent cleanup even if a host tool was removed.
# Resolve tools during run preflight, or lazily inside a recovery operation.
$wsb=$null
$python=$null
. (Join-Path $PSScriptRoot 'gui-native-smoke-process.ps1')

function Api([string]$Path,[string]$Method='GET') {
    $result=& gh.exe api --method $Method $Path
    if ($LASTEXITCODE -ne 0) { throw "GitHub $Method request failed" }
    if ($result) { ($result -join "`n") | ConvertFrom-Json }
}
function Api-Items([string]$Path,[string]$Field,[ValidateRange(1,100)][int]$MaximumPages=100) {
    # Old gh versions support ordinary API requests but not --slurp. Request
    # explicit pages and buffer the complete inventory before exposing any item
    # to registration or cleanup. Truncation and concurrent inventory drift fail.
    if ($Field -notin @('runners','workflow_runs','jobs') -or $Path -match '(?:\?|&)page=') { throw 'Invalid GitHub inventory request' }
    $sizes=[regex]::Matches($Path,'(?:\?|&)per_page=([^&]+)')
    if ($sizes.Count -gt 1 -or ($sizes.Count -eq 1 -and $sizes[0].Groups[1].Value -cne '100')) { throw 'GitHub inventory must use exactly 100 items per page' }
    if ($sizes.Count -eq 0) { $Path += $(if ($Path.Contains('?')) {'&'} else {'?'})+'per_page=100' }
    $items=[Collections.Generic.List[object]]::new()
    $seen=@{}
    $total=$null
    for ($pageNumber=1; $pageNumber -le $MaximumPages; $pageNumber++) {
        $response=Api "$Path&page=$pageNumber"
        if ($null -eq $response -or $null -eq $response.PSObject.Properties[$Field] -or $null -eq $response.PSObject.Properties['total_count']) { throw 'GitHub inventory response is incomplete' }
        $count=$response.total_count
        if (($count -isnot [int] -and $count -isnot [long]) -or $count -lt 0) { throw 'Invalid GitHub inventory count' }
        if ($count -gt (100*$MaximumPages)) { throw 'GitHub inventory exceeds the bounded pagination limit' }
        if ($null -eq $total) { $total=$count }
        elseif ($total -ne $count) { throw 'GitHub inventory changed during pagination; retry the complete observation' }
        if ($response.$Field -isnot [Array]) { throw 'GitHub inventory items must be an array' }
        $pageItems=@($response.$Field)
        if ($pageItems.Count -gt 100) { throw 'GitHub inventory page exceeded its requested size' }
        foreach ($item in $pageItems) {
            if ($null -eq $item -or $null -eq $item.PSObject.Properties['id'] -or ($item.id -isnot [int] -and $item.id -isnot [long]) -or $item.id -le 0) { throw 'Invalid GitHub inventory item identity' }
            $key=[string]$item.id
            if ($seen.ContainsKey($key)) { throw 'GitHub inventory repeated an item across pages' }
            $seen[$key]=$true
            $items.Add($item)
        }
        if ($items.Count -gt $total) { throw 'GitHub inventory exceeded its declared count' }
        if ($items.Count -eq $total) { return $items }
        if ($pageItems.Count -lt 100) { throw 'GitHub inventory ended before its declared count' }
    }
    throw 'GitHub inventory exceeds the bounded pagination limit'
}
function Save-Receipt {
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath "$receiptPath.pending" -Encoding utf8
    Move-Item -LiteralPath "$receiptPath.pending" -Destination $receiptPath -Force
}
function Invoke-Control([string]$Name,[string[]]$Arguments,[int]$Timeout=120000) {
    $controlExe=if ($wsb) { $wsb } else { (Get-Command wsb.exe -ErrorAction Stop).Source }
    $capture=Invoke-CapturedProcess -FilePath $controlExe -Arguments ($Arguments+@('--raw')) -WorkingDirectory $repoRoot `
        -ProcessTimeoutMs $Timeout -StdoutPath "$runRoot\$Name.stdout.log" -StderrPath "$runRoot\$Name.stderr.log"
    if ($capture.exit_code -ne 0 -or $capture.timed_out) { throw "Sandbox $Name failed" }
    Get-Content -LiteralPath "$runRoot\$Name.stdout.log" -Raw
}
function Get-GuestIds([string]$Name) {
    $listing=(Invoke-Control $Name @('list') 15000) | ConvertFrom-Json
    if (-not ($listing.PSObject.Properties.Name -contains 'WindowsSandboxEnvironments')) { throw 'Unrecognized Sandbox inventory' }
    @($listing.WindowsSandboxEnvironments | ForEach-Object { [string]$_.Id })
}
function Export-Diagnostic {
    # Repeated recovery retains each diagnostic attempt, including unavailable
    # evidence when an interrupted guest never completed its private export.
    $safe=Join-Path $runRoot ('safe-evidence-'+[Guid]::NewGuid().ToString())
    $exporterExe=if ($python) { $python } else { (Get-Command python -ErrorAction Stop).Source }
    & $exporterExe (Join-Path $PSScriptRoot 'native_failure_evidence.py') export --root "$runRoot\output" --output $safe `
        --source-sha $receipt.source_sha --run-id ([string]$receipt.run_id) --run-attempt ([string]$receipt.run_attempt) `
        --stage host-fallback --cleanup $(if ($receipt.sandbox_stopped -and $receipt.runner_removed) {'passed'} else {'pending'})
    if ($LASTEXITCODE -ne 0) { $receipt.evidence_export='unavailable' }
    else { $receipt.evidence_export='exported'; $receipt.evidence_directory=Split-Path -Leaf $safe }
}
function Remove-OwnedInstance {
    # Independent cleanup attempts: an API outage must not leave the VM running,
    # and an unavailable Sandbox CLI must not leave a registration active.
    if (Test-Path -LiteralPath $tokenPath) { Remove-Item -LiteralPath $tokenPath -Force }
    if (Test-Path -LiteralPath "$tokenPath.pending") { Remove-Item -LiteralPath "$tokenPath.pending" -Force }
    try {
        Export-Diagnostic
    } catch { $receipt.evidence_export='unavailable' }
    try {
        if (@(Get-GuestIds 'before-stop') -contains $InstanceId.ToString()) {
            $null=Invoke-Control 'stop' @('stop','--id',$InstanceId.ToString())
        }
        $receipt.sandbox_stopped=@(Get-GuestIds 'after-stop') -notcontains $InstanceId.ToString()
    } catch { $receipt.cleanup_errors+= 'sandbox-stop-unconfirmed' }
    try {
        $runners=@(Api-Items "repos/$repo/actions/runners?per_page=100" 'runners')
        $owned=@($runners | Where-Object name -CEQ $runnerName)
        $unregisterClock=[Diagnostics.Stopwatch]::StartNew()
        while ($receipt.status -eq 'passed' -and $owned.Count -gt 0 -and $unregisterClock.Elapsed.TotalSeconds -lt 15) {
            Start-Sleep -Milliseconds 1000
            $runners=@(Api-Items "repos/$repo/actions/runners?per_page=100" 'runners')
            $owned=@($runners | Where-Object name -CEQ $runnerName)
        }
        $receipt.automatic_unregister=$owned.Count -eq 0
        foreach ($runner in $owned) { $null=Api "repos/$repo/actions/runners/$($runner.id)" 'DELETE' }
        $remaining=@(Api-Items "repos/$repo/actions/runners?per_page=100" 'runners')
        $receipt.runner_removed=@($remaining | Where-Object name -CEQ $runnerName).Count -eq 0
    } catch { $receipt.cleanup_errors+= 'runner-unregister-unconfirmed' }
    $receipt.finished_at_utc=[DateTime]::UtcNow.ToString('o')
    Save-Receipt
    # A final separate receipt records actual cleanup, after the diagnostic was
    # exported before teardown. It never rewrites the earlier diagnostic bundle.
    try { Export-Diagnostic; Save-Receipt } catch { $receipt.evidence_export='unavailable'; Save-Receipt }
}

if ($CleanupOnly) {
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) { throw 'Recovery requires the retained instance receipt' }
    $saved=Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    if ($saved.instance -cne $InstanceId.ToString() -or $saved.repository -cne $repo -or $saved.runner_name -cne $runnerName -or $saved.source_sha -notmatch '^[0-9a-f]{40}$') { throw 'Recovery receipt does not identify this owned instance' }
    $receipt=[ordered]@{}
    foreach ($entry in $saved.PSObject.Properties) { $receipt[$entry.Name]=$entry.Value }
    Remove-OwnedInstance
    if (-not $receipt.sandbox_stopped -or -not $receipt.runner_removed) { throw 'Recovery cleanup remains unconfirmed; retry exact instance recovery' }
    return
}

if (Test-Path -LiteralPath $runRoot) { throw 'Instance already exists; use a fresh UUID or recover the exact retained instance' }
$null=New-Item -ItemType Directory -Path "$runRoot\input","$runRoot\output"
$receipt=[ordered]@{
    schema_version=1; kind='sorotte-native-runner-controller'; repository=$repo; instance=$InstanceId.ToString(); runner_name=$runnerName
    source_sha=$SourceSha; run_id=$RunId; run_attempt=$RunAttempt; job_id=$JobId; job_conclusion=$null
    status='preparing'; started_at_utc=[DateTime]::UtcNow.ToString('o'); finished_at_utc=$null
    provisioning_seconds=$null; queue_seconds=$null; execution_seconds=$null
    sandbox_stopped=$false; runner_removed=$false; automatic_unregister=$false
    evidence_export='unavailable'; evidence_directory=$null; cleanup_errors=@(); error=$null
}
Save-Receipt
$connector=$null
$guardian=$null
$clock=[Diagnostics.Stopwatch]::StartNew()
try {
    $wsb=(Get-Command wsb.exe -ErrorAction Stop).Source
    $python=(Get-Command python -ErrorAction Stop).Source
    $bundle=(Resolve-Path -LiteralPath $BundleDirectory).Path
    & $python (Join-Path $PSScriptRoot 'native_runner_bundle.py') validate --bundle $bundle
    if ($LASTEXITCODE -ne 0) { throw 'Reviewed tool bundle validation failed' }
    $manifest=Get-Content -LiteralPath "$bundle\tools-manifest.json" -Raw | ConvertFrom-Json
    $receipt['tools_manifest_sha256']=(Get-FileHash -LiteralPath "$bundle\tools-manifest.json" -Algorithm SHA256).Hash.ToLowerInvariant()
    if (@(Get-GuestIds 'before').Count) { throw 'An existing Sandbox is running; it must not be disturbed' }
    Copy-Item -LiteralPath "$bundle\tools-manifest.json" -Destination "$runRoot\input"
    foreach ($name in @('native-runner-guest.ps1','native_failure_evidence.py','gui-native-smoke-process.ps1')) {
        Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination "$runRoot\input"
    }
    $hashes=@{}
    foreach ($name in @('native-runner-guest.ps1','native_failure_evidence.py','gui-native-smoke-process.ps1','tools-manifest.json')) {
        $hashes[$name]=(Get-FileHash -LiteralPath "$runRoot\input\$name" -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $config=[xml]'<Configuration><vGPU>Enable</vGPU><MemoryInMB>16384</MemoryInMB><Networking>Enable</Networking><ClipboardRedirection>Disable</ClipboardRedirection><AudioInput>Disable</AudioInput><VideoInput>Disable</VideoInput><PrinterRedirection>Disable</PrinterRedirection><MappedFolders/><LogonCommand><Command/></LogonCommand></Configuration>'
    foreach ($mapping in @(@("$runRoot\input",'C:\SorotteCIBootstrap','true'),@("$bundle\tools",'C:\SorotteCIToolsSource','true'),@("$runRoot\output",'C:\SorotteCIOutput','false'))) {
        $folder=$config.CreateElement('MappedFolder')
        for ($i=0;$i -lt 3;$i++) { $node=$config.CreateElement(@('HostFolder','SandboxFolder','ReadOnly')[$i]); $node.InnerText=$mapping[$i]; $null=$folder.AppendChild($node) }
        $null=$config.Configuration.SelectSingleNode('MappedFolders').AppendChild($folder)
    }
    $config.Configuration.LogonCommand.Command="powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File C:\SorotteCIBootstrap\native-runner-guest.ps1 -InstanceId $InstanceId -ToolsManifestSha256 $($hashes['tools-manifest.json']) -ScriptSha256 $($hashes['native-runner-guest.ps1']) -HelperSha256 $($hashes['gui-native-smoke-process.ps1']) -ExporterSha256 $($hashes['native_failure_evidence.py'])"
    $config.Save("$runRoot\run.wsb")
    Save-Receipt
    # Guardian survives controller interruption and knows only the owned UUID.
    $guardianArgs=@('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'native-runner-watchdog.ps1'),'-ControllerPid',[string]$PID,'-ControllerStartUtc',(Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o'),'-InstanceId',$InstanceId.ToString(),'-TimeoutMinutes',[string]$TimeoutMinutes)
    $guardian=Start-Process -FilePath powershell.exe -ArgumentList (($guardianArgs | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join ' ') -WindowStyle Hidden -PassThru
    $null=Invoke-Control 'start' @('start','--id',$InstanceId.ToString(),'--config',$config.OuterXml)
    if (@(Get-GuestIds 'started') -notcontains $InstanceId.ToString()) { throw 'Requested isolated guest did not start' }
    $connectArgs=@('connect','--id',$InstanceId.ToString(),'--raw')
    $connector=Start-Process -FilePath $wsb -ArgumentList (($connectArgs | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join ' ') -WindowStyle Hidden -PassThru
    Write-Host "Guest provisioned under instance $InstanceId; waiting for harmless tool and desktop preflight."
    while (-not (Test-Path -LiteralPath "$runRoot\output\ready.json")) {
        if ((Test-Path -LiteralPath "$runRoot\output\completion.json") -or $clock.Elapsed.TotalMinutes -gt 25) { throw 'Guest preparation failed or expired before registration' }
        Start-Sleep -Milliseconds 500
    }
    $ready=Get-Content -LiteralPath "$runRoot\output\ready.json" -Raw | ConvertFrom-Json
    if ($ready.instance -cne $InstanceId.ToString() -or $ready.status -cne 'ready') { throw 'Guest readiness identity mismatch' }
    $receipt.provisioning_seconds=$clock.Elapsed.TotalSeconds
    $assignment=@{source_sha=$SourceSha;run_id=$RunId;run_attempt=$RunAttempt;job_id=$JobId}
    $assignment | ConvertTo-Json | Set-Content -LiteralPath "$runRoot\assignment.json" -Encoding utf8
    $run=Api "repos/$repo/actions/runs/$RunId"
    $job=Api "repos/$repo/actions/jobs/$JobId"
    $run | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath "$runRoot\authorized-run.json" -Encoding utf8
    $job | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath "$runRoot\authorized-job.json" -Encoding utf8
    & $python (Join-Path $PSScriptRoot 'native_runner_bundle.py') validate-assignment --bundle $bundle --assignment "$runRoot\assignment.json" --run "$runRoot\authorized-run.json" --job "$runRoot\authorized-job.json"
    if ($LASTEXITCODE -ne 0) { throw 'Trusted workflow/run/job authorization failed' }
    $runners=@(Api-Items "repos/$repo/actions/runners?per_page=100" 'runners')
    if (@($runners | Where-Object { @($_.labels | Where-Object name -CEQ 'sorotte-native-interactive').Count }).Count) { throw 'Another native runner is registered' }
    foreach ($status in @('queued','in_progress')) {
        foreach ($other in @(Api-Items "repos/$repo/actions/runs?status=$status&per_page=100" 'workflow_runs')) {
            foreach ($queued in @(Api-Items "repos/$repo/actions/runs/$($other.id)/jobs?per_page=100" 'jobs')) {
                if ($queued.status -eq 'queued' -and $queued.labels -contains 'sorotte-native-interactive' -and $queued.id -ne $JobId) { throw 'Another queued native job could claim this registration' }
            }
        }
    }
    $registration=Api "repos/$repo/actions/runners/registration-token" 'POST'
    @{instance=$InstanceId.ToString();candidate=$SourceSha;run=$RunId;run_attempt=$RunAttempt;repository=$repo;token=$registration.token;expires_at=$registration.expires_at} | ConvertTo-Json | Set-Content -LiteralPath "$tokenPath.pending" -Encoding utf8
    Move-Item -LiteralPath "$tokenPath.pending" -Destination $tokenPath
    $registration=$null
    $receipt.status='running-job'; Save-Receipt
    $queueClock=[Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath "$runRoot\output\completion.json")) {
        if ($null -eq $receipt.queue_seconds -and (Test-Path -LiteralPath "$runRoot\output\assigned-job.json")) { $receipt.queue_seconds=$queueClock.Elapsed.TotalSeconds; Save-Receipt }
        if ($clock.Elapsed.TotalMinutes -gt $TimeoutMinutes) { throw 'Assigned native job exceeded controller deadline' }
        Start-Sleep -Milliseconds 500
    }
    $completion=Get-Content -LiteralPath "$runRoot\output\completion.json" -Raw | ConvertFrom-Json
    $job=Api "repos/$repo/actions/jobs/$JobId"
    if ($completion.instance -cne $InstanceId.ToString() -or $completion.candidate -cne $SourceSha -or $completion.run -ne $RunId -or $completion.run_attempt -ne $RunAttempt -or $completion.status -cne 'job-finished' -or $completion.runner_exit_code -ne 0 -or $job.runner_name -cne $runnerName -or $job.status -cne 'completed') { throw 'Guest or GitHub did not attest the exact completed one-job assignment' }
    $receipt.job_conclusion=$job.conclusion
    if ($job.started_at -and $job.completed_at) { $receipt.execution_seconds=([DateTime]::Parse($job.completed_at)-[DateTime]::Parse($job.started_at)).TotalSeconds }
    if ($job.conclusion -cne 'success') { throw 'Assigned native job failed; retained diagnostic evidence identifies this attempt' }
    $receipt.status='passed'
} catch { $receipt.status='failed'; $receipt.error=$_.Exception.Message }
finally {
    if (Test-Path -LiteralPath "$tokenPath.pending") { Remove-Item -LiteralPath "$tokenPath.pending" -Force }
    Remove-OwnedInstance
    if ($null -ne $connector) { $connector.Dispose() }
    if ($null -ne $guardian) { $guardian.Dispose() }
    Write-Host "Native runner receipt: $receiptPath"
}
if ($receipt.status -cne 'passed' -or -not $receipt.sandbox_stopped -or -not $receipt.runner_removed -or -not $receipt.automatic_unregister -or $receipt.evidence_export -cne 'exported') { exit 1 }
