function ConvertTo-ProcessArgument {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') {
        return $Value
    }

    $builder = [System.Text.StringBuilder]::new()
    $null = $builder.Append([char]34)
    $backslashCount = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]92) {
            $backslashCount += 1
            continue
        }
        if ($character -eq [char]34) {
            for ($slash = 0; $slash -lt (($backslashCount * 2) + 1); $slash += 1) {
                $null = $builder.Append([char]92)
            }
            $null = $builder.Append([char]34)
            $backslashCount = 0
            continue
        }
        for ($slash = 0; $slash -lt $backslashCount; $slash += 1) {
            $null = $builder.Append([char]92)
        }
        $backslashCount = 0
        $null = $builder.Append($character)
    }
    for ($slash = 0; $slash -lt ($backslashCount * 2); $slash += 1) {
        $null = $builder.Append([char]92)
    }
    $null = $builder.Append([char]34)
    return $builder.ToString()
}

function Get-CapturedTaskText {
    param(
        [Parameter(Mandatory = $true)]$Task,
        [Parameter(Mandatory = $true)][int]$DrainTimeoutMs
    )

    try {
        if ($Task.Wait($DrainTimeoutMs)) {
            return $Task.Result
        }
        return ""
    }
    catch {
        return ""
    }
}

function Invoke-CapturedProcess {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$StdoutPath,
        [Parameter(Mandatory = $true)][string]$StderrPath,
        [Parameter(Mandatory = $true)][int]$ProcessTimeoutMs,
        [hashtable]$EnvironmentVariables = @{}
    )

    $processStart = [System.Diagnostics.ProcessStartInfo]::new()
    $processStart.FileName = $FilePath
    $processStart.Arguments = (($Arguments | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join " ")
    $processStart.WorkingDirectory = $WorkingDirectory
    $processStart.UseShellExecute = $false
    $processStart.CreateNoWindow = $true
    $processStart.RedirectStandardOutput = $true
    $processStart.RedirectStandardError = $true
    $processStart.StandardOutputEncoding = [System.Text.Encoding]::UTF8
    $processStart.StandardErrorEncoding = [System.Text.Encoding]::UTF8
    foreach ($name in $EnvironmentVariables.Keys) {
        $processStart.EnvironmentVariables[[string]$name] = [string]$EnvironmentVariables[$name]
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $processStart
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $timedOut = $false
    $treeKillExitCode = $null
    $taskkillOutput = ""
    $taskkillError = ""
    $startError = $null
    try {
        try {
            $started = $process.Start()
        }
        catch {
            $started = $false
            $startError = $_.Exception.Message
        }
        if (-not $started) {
            if (-not $startError) {
                $startError = "process Start returned false"
            }
            $utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
            [System.IO.File]::WriteAllText($StdoutPath, "", $utf8WithoutBom)
            [System.IO.File]::WriteAllText(
                $StderrPath,
                "HARNESS_START_FAILURE: $FilePath`: $startError`n",
                $utf8WithoutBom
            )
            $stopwatch.Stop()
            return [pscustomobject]@{
                exit_code = 125
                timed_out = $false
                duration_ms = $stopwatch.ElapsedMilliseconds
                process_timeout_ms = $ProcessTimeoutMs
                tree_kill_exit_code = $null
                start_error = $startError
            }
        }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($ProcessTimeoutMs)) {
            $timedOut = $true
            $taskkillStart = [System.Diagnostics.ProcessStartInfo]::new()
            $taskkillStart.FileName = Join-Path $env:SystemRoot "System32\taskkill.exe"
            $taskkillStart.Arguments = "/PID $($process.Id) /T /F"
            $taskkillStart.UseShellExecute = $false
            $taskkillStart.CreateNoWindow = $true
            $taskkillStart.RedirectStandardOutput = $true
            $taskkillStart.RedirectStandardError = $true
            $taskkill = [System.Diagnostics.Process]::new()
            $taskkill.StartInfo = $taskkillStart
            try {
                if ($taskkill.Start()) {
                    $taskkillOutputTask = $taskkill.StandardOutput.ReadToEndAsync()
                    $taskkillErrorTask = $taskkill.StandardError.ReadToEndAsync()
                    if (-not $taskkill.WaitForExit(15000)) {
                        $taskkill.Kill()
                        $null = $taskkill.WaitForExit(5000)
                    }
                    $taskkillOutput = Get-CapturedTaskText `
                        -Task $taskkillOutputTask `
                        -DrainTimeoutMs 5000
                    $taskkillError = Get-CapturedTaskText `
                        -Task $taskkillErrorTask `
                        -DrainTimeoutMs 5000
                    $treeKillExitCode = if ($taskkill.HasExited) {
                        $taskkill.ExitCode
                    }
                    else {
                        -1
                    }
                }
                else {
                    $taskkillError = "taskkill did not start"
                    $treeKillExitCode = -1
                }
            }
            catch {
                $taskkillError = "taskkill failed: $($_.Exception.Message)"
                $treeKillExitCode = -1
            }
            finally {
                $taskkill.Dispose()
            }
            if (-not $process.WaitForExit(10000)) {
                try {
                    $process.Kill()
                    if (-not $process.WaitForExit(10000)) {
                        $taskkillError += " root process remained alive after Kill"
                    }
                }
                catch {
                    $taskkillError += " root kill failed: $($_.Exception.Message)"
                }
            }
        }
        else {
            # The parameterized WaitForExit overload can return before async
            # stream callbacks have drained; the parameterless call completes
            # that drain only after the bounded process wait succeeded.
            $process.WaitForExit()
        }
        $stdout = Get-CapturedTaskText -Task $stdoutTask -DrainTimeoutMs 10000
        $stderr = Get-CapturedTaskText -Task $stderrTask -DrainTimeoutMs 10000
        if (-not $stdoutTask.IsCompleted -or -not $stderrTask.IsCompleted) {
            $taskkillError += " redirected stream capture did not drain"
        }
        if ($timedOut) {
            $stderr += (
                "`nHARNESS_TIMEOUT: process exceeded {0} ms; pid={1}; " +
                "tree_kill_exit={2}; taskkill_stdout={3}; taskkill_stderr={4}`n"
            ) -f (
                $ProcessTimeoutMs,
                $process.Id,
                $treeKillExitCode,
                ($taskkillOutput -replace "\r?\n", " ").Trim(),
                ($taskkillError -replace "\r?\n", " ").Trim()
            )
        }
        $utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
        [System.IO.File]::WriteAllText($StdoutPath, $stdout, $utf8WithoutBom)
        [System.IO.File]::WriteAllText($StderrPath, $stderr, $utf8WithoutBom)
        $stopwatch.Stop()
        return [pscustomobject]@{
            exit_code = if ($timedOut) { 124 } else { $process.ExitCode }
            timed_out = $timedOut
            duration_ms = $stopwatch.ElapsedMilliseconds
            process_timeout_ms = $ProcessTimeoutMs
            tree_kill_exit_code = $treeKillExitCode
            start_error = $null
        }
    }
    finally {
        $stopwatch.Stop()
        $process.Dispose()
    }
}
