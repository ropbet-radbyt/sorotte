# Cleanup uses only read-only run/job observations and narrowly bound cancel or
# unregister requests. It must not depend on Python, the guest, or a responsive
# network. API captures stay in the private instance directory; the safe exporter
# deliberately does not publish them.
function Invoke-NativeCleanupCommand {
    param([string]$FilePath,[string[]]$Arguments,[int]$TimeoutMs)
    $start=[Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$FilePath
    $start.Arguments=(($Arguments | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join ' ')
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true
    $start.RedirectStandardError=$true
    $start.StandardOutputEncoding=[Text.UTF8Encoding]::new($false,$true)
    $start.StandardErrorEncoding=[Text.UTF8Encoding]::new($false,$true)
    $child=[Diagnostics.Process]::new()
    $child.StartInfo=$start
    try {
        if (-not $child.Start()) { throw 'Cleanup API process did not start' }
        # Retain the handle: termination cannot accidentally target a reused PID.
        $null=$child.Handle
        $stdout=$child.StandardOutput.ReadToEndAsync()
        $stderr=$child.StandardError.ReadToEndAsync()
        $expired=-not $child.WaitForExit($TimeoutMs)
        if ($expired) {
            if (-not $child.HasExited) { $child.Kill() }
            if (-not $child.WaitForExit(1000)) { throw 'Owned cleanup API process did not stop after its deadline' }
        }
        # Never use parameterless WaitForExit or unbounded redirected-stream waits.
        if (-not $stdout.Wait(1000) -or -not $stderr.Wait(1000)) { throw 'Cleanup API output did not drain within its bound' }
        return @{exit_code=$child.ExitCode;timed_out=$expired;stdout=$stdout.Result;stderr=$stderr.Result}
    } finally {
        try { if ($null -ne $child.SafeHandle -and -not $child.HasExited) { $child.Kill(); $null=$child.WaitForExit(1000) } } catch { }
        $child.Dispose()
    }
}

function New-NativeCleanupContext([string]$Directory,[int]$BudgetMs,[string]$ReceiptPath) {
    $null=New-Item -ItemType Directory -Path $Directory
    return @{
        directory=$Directory;receipt_path=$ReceiptPath;clock=[Diagnostics.Stopwatch]::StartNew();budget_ms=$BudgetMs
        call_timeout_ms=10000;cancel_grace_ms=30000;force_grace_ms=30000
        unregister_grace_ms=15000;poll_ms=1000;sequence=0
    }
}

function Invoke-NativeCleanupApi($Context,[string]$Path,[string]$Method='GET') {
    $remaining=$Context.budget_ms-$Context.clock.ElapsedMilliseconds
    if ($remaining -le 0) { throw 'Native cleanup API budget expired' }
    $Context.sequence++
    $capture=[ordered]@{method=$Method;path=$Path;observed_at_utc=[DateTime]::UtcNow.ToString('o');result=$null;error=$null}
    try {
        $result=Invoke-NativeCleanupCommand -FilePath (Get-Command gh.exe -ErrorAction Stop).Source `
            -Arguments @('api','--method',$Method,$Path) -TimeoutMs ([Math]::Min($remaining,$Context.call_timeout_ms))
        $capture.result=$result
        if ($result.timed_out) { throw "GitHub cleanup $Method request timed out" }
        if ($result.exit_code -ne 0) { throw "GitHub cleanup $Method request failed: $($result.stderr.Trim())" }
        if ($result.stdout) { return ($result.stdout | ConvertFrom-Json) }
    } catch { $capture.error=$_.Exception.Message; throw }
    finally { Write-NativeRunnerReceipt -Path (Join-Path $Context.directory ('api-'+$Context.sequence+'.json')) -Value $capture }
}

function Get-NativeCleanupRunners($Context,[string]$Repository) {
    $items=[Collections.Generic.List[object]]::new()
    $seen=@{}
    $total=$null
    for ($page=1;$page -le 100;$page++) {
        $response=Invoke-NativeCleanupApi $Context "repos/$Repository/actions/runners?per_page=100&page=$page"
        if ($null -eq $response -or $response.total_count -isnot [int] -or $response.total_count -lt 0 -or $response.total_count -gt 10000 -or $response.runners -isnot [Array]) { throw 'Invalid cleanup runner inventory' }
        if ($null -ne $total -and $total -ne $response.total_count) { throw 'Cleanup runner inventory changed during pagination' }
        $total=$response.total_count
        if ($response.runners.Count -gt 100) { throw 'Cleanup runner page exceeded its bound' }
        foreach ($runner in $response.runners) {
            if (($runner.id -isnot [int] -and $runner.id -isnot [long]) -or $runner.id -le 0 -or $seen.ContainsKey([string]$runner.id)) { throw 'Invalid or repeated cleanup runner identity' }
            $seen[[string]$runner.id]=$true
            $items.Add($runner)
        }
        if ($items.Count -eq $total) { return $items }
        if ($items.Count -gt $total -or $response.runners.Count -lt 100) { throw 'Truncated cleanup runner inventory' }
    }
    throw 'Cleanup runner inventory exceeded its bound'
}

function Get-NativeCleanupAssignment($Context,$Receipt) {
    $repository=$Receipt.repository
    if ($repository -cne 'ropbet-radbyt/sorotte' -or $Receipt.source_sha -cnotmatch '^[0-9a-f]{40}$' -or $Receipt.runner_name -cne ('sorotte-sandbox-'+$Receipt.instance)) { throw 'Cleanup receipt ownership mismatch' }
    foreach ($key in @('run_id','run_attempt','job_id')) {
        if (($Receipt[$key] -isnot [int] -and $Receipt[$key] -isnot [long]) -or $Receipt[$key] -le 0) { throw 'Invalid cleanup run/attempt/job identity' }
    }
    $run=Invoke-NativeCleanupApi $Context "repos/$repository/actions/runs/$($Receipt.run_id)"
    $job=Invoke-NativeCleanupApi $Context "repos/$repository/actions/jobs/$($Receipt.job_id)"
    $profile=Get-Content -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'verification/windows-native-guest.json') -Raw | ConvertFrom-Json
    if ($run.id -ne $Receipt.run_id -or $run.run_attempt -ne $Receipt.run_attempt -or $run.head_sha -cne $Receipt.source_sha -or
        $run.repository.full_name -cne $repository -or $run.head_repository.full_name -cne $repository -or
        $run.path -cnotin $profile.allowed_workflows -or $run.event -cnotin @('workflow_dispatch','push','schedule') -or
        $job.id -ne $Receipt.job_id -or $job.run_id -ne $Receipt.run_id -or $job.run_attempt -ne $Receipt.run_attempt -or $job.head_sha -cne $Receipt.source_sha) { throw 'Cleanup workflow source, attempt, or job ownership changed' }
    foreach ($label in @('self-hosted','Windows','X64','sorotte-native-interactive','sorotte-ephemeral')) {
        if ($label -cnotin $job.labels) { throw 'Cleanup job does not identify the isolated native contract' }
    }
    $assigned=$job.runner_id -gt 0 -and $job.runner_name -ceq $Receipt.runner_name
    $unassigned=$job.runner_id -eq 0 -and -not $job.runner_name -and $job.status -cin @('queued','completed')
    if (-not $assigned -and -not $unassigned) { throw 'Cleanup job was assigned to a different runner' }
    $owned=@(Get-NativeCleanupRunners $Context $repository | Where-Object name -CEQ $Receipt.runner_name)
    if ($owned.Count -gt 1) { throw 'Cleanup runner name is ambiguous' }
    if ($owned.Count -eq 1 -and ($owned[0].busy -isnot [bool] -or ($assigned -and $owned[0].id -ne $job.runner_id) -or ($unassigned -and $owned[0].busy))) { throw 'Cleanup runner identity or busy assignment changed' }
    $snapshot=@{run=$run;job=$job;owned=$owned;drained=($job.status -ceq 'completed' -and ($owned.Count -eq 0 -or -not $owned[0].busy))}
    Write-NativeRunnerReceipt -Path (Join-Path $Context.directory ('assignment-'+$Context.sequence+'.json')) -Value $snapshot
    return $snapshot
}

function Confirm-NativeCleanupAttempt($Context,$Receipt,$State) {
    # Pagination/job lookup may take time. Read the mutable current attempt last,
    # directly before a run-id mutation; GitHub offers no attempt-specific cancel.
    $current=Invoke-NativeCleanupApi $Context "repos/$($Receipt.repository)/actions/runs/$($Receipt.run_id)"
    if ($current.id -ne $Receipt.run_id -or $current.run_attempt -ne $Receipt.run_attempt -or
        $current.head_sha -cne $Receipt.source_sha -or $current.path -cne $State.run.path -or
        $current.repository.full_name -cne $Receipt.repository -or $current.head_repository.full_name -cne $Receipt.repository) { throw 'Cleanup current workflow attempt changed before mutation' }
    $State.run=$current
}

function Stop-NativeQualificationRequest($Context,$Request) {
    # Source-only discovery/adoption is not permission to cancel another
    # controller's work. The qualifier must have bound a queued job and its own
    # fresh instance before entering provisioning.
    if (-not $Request.instance -or $Request.instance -cnotmatch '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$') { throw 'Native request has no bound isolated instance' }
    $authority=@{
        repository='ropbet-radbyt/sorotte';source_sha=$Request.source_sha
        run_id=$Request.run_id;run_attempt=$Request.run_attempt;job_id=$Request.job_id
        instance=$Request.instance;runner_name='sorotte-sandbox-'+$Request.instance
    }
    $state=Get-NativeCleanupAssignment $Context $authority
    if ($state.run.path -cne '.github/workflows/gui-native-interactive.yml') { throw 'Native request cancellation workflow changed' }
    if ($state.run.status -ceq 'completed' -or $state.job.status -ceq 'completed') { return 'already-completed' }
    Confirm-NativeCleanupAttempt $Context $authority $state
    if ($state.run.status -ceq 'completed') { return 'already-completed' }
    try {
        $null=Invoke-NativeCleanupApi $Context "repos/ropbet-radbyt/sorotte/actions/runs/$($Request.run_id)/cancel" 'POST'
        return 'requested'
    } catch {
        if ($_.Exception.Message -notmatch 'HTTP 409') { throw }
        return 'conflict-awaiting-controller-drain'
    }
}

function Wait-NativeCleanupDrain($Context,$Receipt,[int]$GraceMs) {
    $clock=[Diagnostics.Stopwatch]::StartNew()
    do {
        $state=Get-NativeCleanupAssignment $Context $Receipt
        if ($state.drained) { return $state }
        Start-Sleep -Milliseconds $Context.poll_ms
    } while ($clock.ElapsedMilliseconds -lt $GraceMs -and $Context.clock.ElapsedMilliseconds -lt $Context.budget_ms)
    return $state
}

function Stop-NativeOwnedJob($Context,$Receipt) {
    $state=Get-NativeCleanupAssignment $Context $Receipt
    if ($Receipt.status -ceq 'passed') {
        if ($state.job.status -cne 'completed' -or $state.job.conclusion -cne 'success') { throw 'Passing receipt no longer identifies a successful completed job' }
        # Leave a completed runner alive long enough to acknowledge ephemeral
        # unregister. Never cancel the enclosing release workflow after success.
        $clock=[Diagnostics.Stopwatch]::StartNew()
        while ($state.owned.Count -gt 0 -and $clock.ElapsedMilliseconds -lt $Context.unregister_grace_ms) {
            Start-Sleep -Milliseconds $Context.poll_ms
            $state=Get-NativeCleanupAssignment $Context $Receipt
        }
        $Receipt.automatic_unregister=$state.owned.Count -eq 0 -and -not ($Receipt.Contains('runner_delete_requested') -and $Receipt.runner_delete_requested)
        return
    }
    if ($state.drained) { $Receipt['job_drain']='already-completed'; return }
    # Repeat every source/run/attempt/job/runner check immediately before either
    # run-id mutation. A newer attempt must never be cancelled by old recovery.
    $state=Get-NativeCleanupAssignment $Context $Receipt
    if (-not $state.drained -and $state.run.status -cne 'completed') {
        Confirm-NativeCleanupAttempt $Context $Receipt $state
        if ($state.run.status -cne 'completed') {
            try {
                $null=Invoke-NativeCleanupApi $Context "repos/$($Receipt.repository)/actions/runs/$($Receipt.run_id)/cancel" 'POST'
                $Receipt['job_drain']='normal-cancel-requested'
            } catch {
                # The qualifier can already have cancelled this exact run while
                # the watchdog starts. A 409 is an observation race, not proof of
                # terminal state: preserve it and give the live guest its window.
                if ($_.Exception.Message -notmatch 'HTTP 409') { throw }
                $Receipt['job_drain']='normal-cancel-conflict'
            }
        }
    }
    $state=Wait-NativeCleanupDrain $Context $Receipt $Context.cancel_grace_ms
    if (-not $state.drained -and $Receipt.job_drain -cin @('normal-cancel-requested','normal-cancel-conflict')) {
        $state=Get-NativeCleanupAssignment $Context $Receipt
        if (-not $state.drained -and $state.run.status -cne 'completed') {
            Confirm-NativeCleanupAttempt $Context $Receipt $state
            if ($state.run.status -cne 'completed') {
                try {
                    $null=Invoke-NativeCleanupApi $Context "repos/$($Receipt.repository)/actions/runs/$($Receipt.run_id)/force-cancel" 'POST'
                    $Receipt['job_drain']='force-cancel-requested'
                } catch {
                    if ($_.Exception.Message -notmatch 'HTTP 409') { throw }
                    $Receipt['job_drain']='force-cancel-conflict'
                }
            }
        }
        $state=Wait-NativeCleanupDrain $Context $Receipt $Context.force_grace_ms
    }
    if (-not $state.drained) { throw 'Exact owned job did not drain within the cleanup deadline' }
    $Receipt['job_drain']='completed'
}

function Unregister-NativeOwnedRunner($Context,$Receipt) {
    $clock=[Diagnostics.Stopwatch]::StartNew()
    do {
        $state=Get-NativeCleanupAssignment $Context $Receipt
        if ($state.owned.Count -eq 0) {
            $Receipt.runner_removed=$true
            return
        }
        if ($state.drained) {
            # Refresh binding and idle state immediately before DELETE too.
            $state=Get-NativeCleanupAssignment $Context $Receipt
            if ($state.owned.Count -eq 0) { $Receipt.runner_removed=$true; return }
            if ($state.drained) {
                Confirm-NativeCleanupAttempt $Context $Receipt $state
                # Persist before the mutation: an interruption or later recovery
                # must not turn manual removal into automatic unregister proof.
                $Receipt['runner_delete_requested']=$true
                $Receipt.automatic_unregister=$false
                Write-NativeRunnerReceipt -Path $Context.receipt_path -Value $Receipt
                try { $null=Invoke-NativeCleanupApi $Context "repos/$($Receipt.repository)/actions/runners/$($state.owned[0].id)" 'DELETE' }
                catch {
                    # A busy-state propagation race is retryable only after a
                    # completely fresh binding and idle observation next loop.
                    if ($_.Exception.Message -notmatch 'HTTP 422') { throw }
                }
            }
        }
        Start-Sleep -Milliseconds $Context.poll_ms
    } while ($clock.ElapsedMilliseconds -lt $Context.unregister_grace_ms -and $Context.clock.ElapsedMilliseconds -lt $Context.budget_ms)
    throw 'Exact owned runner removal was not confirmed within the cleanup deadline'
}
