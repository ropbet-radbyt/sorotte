[CmdletBinding()]
param(
    [string]$Executable,
    [string[]]$CommandArgs = @(),
    [ValidateRange(1, 100000)]
    [int]$Iterations = 1,
    [ValidateRange(50, 60000)]
    [int]$SampleMs = 500,
    [ValidateRange(0, 600000)]
    [int]$CooldownMs = 250,
    [ValidateRange(0, 600000)]
    [int]$LingerMs = 1500,
    [ValidateRange(0, 86400)]
    [int]$DurationSeconds = 0,
    [string[]]$ProcessPatterns = @("cargo", "rustc", "mpv", "sorotte*"),
    [string]$CsvPath,
    [switch]$IncludeSystemMemory,
    [switch]$ShowLiveSamples,
    [switch]$StopOnFailure
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-MatchingProcesses {
    param([string[]]$Patterns)

    $all = @(Get-Process -ErrorAction SilentlyContinue)
    if (@($Patterns).Count -eq 0) {
        return @()
    }

    $matched = foreach ($proc in $all) {
        $name = $proc.ProcessName
        $isMatch = $false
        foreach ($pattern in $Patterns) {
            if ($name -like $pattern) {
                $isMatch = $true
                break
            }
        }
        if (-not $isMatch) { continue }

        [pscustomobject]@{
            Timestamp    = [DateTimeOffset]::Now
            ProcessName  = $proc.ProcessName
            Id           = $proc.Id
            WorkingSetMB = [math]::Round(($proc.WorkingSet64 / 1MB), 1)
            PrivateMB    = [math]::Round(($proc.PrivateMemorySize64 / 1MB), 1)
            PagedMB      = [math]::Round(($proc.PagedMemorySize64 / 1MB), 1)
            Handles      = $proc.Handles
        }
    }

    @(
        $matched | Sort-Object `
            @{ Expression = "PrivateMB"; Descending = $true }, `
            @{ Expression = "ProcessName"; Descending = $false }, `
            @{ Expression = "Id"; Descending = $false }
    )
}

function Get-SystemMemorySnapshot {
    try {
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
        $totalMb = [math]::Round(($os.TotalVisibleMemorySize / 1024.0), 1)
        $freeMb = [math]::Round(($os.FreePhysicalMemory / 1024.0), 1)
        [pscustomobject]@{
            TotalMB = $totalMb
            FreeMB  = $freeMb
            UsedMB  = [math]::Round(($totalMb - $freeMb), 1)
        }
    } catch {
        $null
    }
}

function Update-Peaks {
    param(
        [System.Collections.IDictionary]$PeakByPid,
        [object[]]$Processes
    )

    foreach ($p in $Processes) {
        $key = "$($p.ProcessName)#$($p.Id)"
        if (-not $PeakByPid.Contains($key)) {
            $PeakByPid[$key] = [pscustomobject]@{
                ProcessName       = $p.ProcessName
                Id                = $p.Id
                PeakWorkingSetMB  = $p.WorkingSetMB
                PeakPrivateMB     = $p.PrivateMB
                PeakPagedMB       = $p.PagedMB
                PeakHandles       = $p.Handles
                FirstSeen         = $p.Timestamp
                LastSeen          = $p.Timestamp
                Samples           = 1
            }
            continue
        }

        $peak = $PeakByPid[$key]
        if ($p.WorkingSetMB -gt $peak.PeakWorkingSetMB) { $peak.PeakWorkingSetMB = $p.WorkingSetMB }
        if ($p.PrivateMB -gt $peak.PeakPrivateMB) { $peak.PeakPrivateMB = $p.PrivateMB }
        if ($p.PagedMB -gt $peak.PeakPagedMB) { $peak.PeakPagedMB = $p.PagedMB }
        if ($p.Handles -gt $peak.PeakHandles) { $peak.PeakHandles = $p.Handles }
        $peak.LastSeen = $p.Timestamp
        $peak.Samples += 1
    }
}

function Write-SampleRowsToCsv {
    param(
        [string]$Path,
        [int]$Iteration,
        [object[]]$Processes,
        [object]$SystemMemory
    )

    if ([string]::IsNullOrWhiteSpace($Path)) { return }

    $rows = foreach ($p in $Processes) {
        [pscustomobject]@{
            Timestamp    = $p.Timestamp.ToString("o")
            Iteration    = $Iteration
            ProcessName  = $p.ProcessName
            Id           = $p.Id
            WorkingSetMB = $p.WorkingSetMB
            PrivateMB    = $p.PrivateMB
            PagedMB      = $p.PagedMB
            Handles      = $p.Handles
            SystemUsedMB = if ($null -ne $SystemMemory) { $SystemMemory.UsedMB } else { $null }
            SystemFreeMB = if ($null -ne $SystemMemory) { $SystemMemory.FreeMB } else { $null }
            SystemTotalMB = if ($null -ne $SystemMemory) { $SystemMemory.TotalMB } else { $null }
        }
    }

    if (-not $rows) {
        $rows = @(
            [pscustomobject]@{
                Timestamp     = [DateTimeOffset]::Now.ToString("o")
                Iteration     = $Iteration
                ProcessName   = "<none>"
                Id            = -1
                WorkingSetMB  = 0
                PrivateMB     = 0
                PagedMB       = 0
                Handles       = 0
                SystemUsedMB  = if ($null -ne $SystemMemory) { $SystemMemory.UsedMB } else { $null }
                SystemFreeMB  = if ($null -ne $SystemMemory) { $SystemMemory.FreeMB } else { $null }
                SystemTotalMB = if ($null -ne $SystemMemory) { $SystemMemory.TotalMB } else { $null }
            }
        )
    }

    $append = Test-Path -LiteralPath $Path
    $rows | Export-Csv -LiteralPath $Path -NoTypeInformation -Append:$append
}

function Get-LeftoverWatchProcesses {
    param(
        [string[]]$Patterns,
        [int[]]$BaselinePids
    )

    $baseline = @{}
    foreach ($pid in $BaselinePids) { $baseline[$pid] = $true }

    @(Get-MatchingProcesses -Patterns $Patterns | Where-Object { -not $baseline.Contains($_.Id) })
}

function Start-MonitoredProcess {
    param(
        [string]$FilePath,
        [string[]]$ProcessArgs
    )

    if (@($ProcessArgs).Count -gt 0) {
        return Start-Process -FilePath $FilePath -ArgumentList $ProcessArgs -NoNewWindow -PassThru
    }
    Start-Process -FilePath $FilePath -NoNewWindow -PassThru
}

function Run-MemoryMonitor {
    param(
        [int]$Iteration,
        [System.Diagnostics.Process]$CommandProcess,
        [int]$MaxDurationSeconds
    )

    $peakByPid = @{}
    $maxSystemUsedMb = 0.0
    $minSystemFreeMb = [double]::PositiveInfinity
    $start = [DateTimeOffset]::Now

    while ($true) {
        $procs = Get-MatchingProcesses -Patterns $ProcessPatterns
        $systemMem = $null
        if ($IncludeSystemMemory) {
            $systemMem = Get-SystemMemorySnapshot
            if ($null -ne $systemMem) {
                if ($systemMem.UsedMB -gt $maxSystemUsedMb) { $maxSystemUsedMb = $systemMem.UsedMB }
                if ($systemMem.FreeMB -lt $minSystemFreeMb) { $minSystemFreeMb = $systemMem.FreeMB }
            }
        }

        Update-Peaks -PeakByPid $peakByPid -Processes $procs
        Write-SampleRowsToCsv -Path $CsvPath -Iteration $Iteration -Processes $procs -SystemMemory $systemMem

        if ($ShowLiveSamples) {
            $stamp = [DateTimeOffset]::Now.ToString("HH:mm:ss.fff")
            if (@($procs).Count -eq 0) {
                Write-Host "[$stamp] no matching processes"
            } else {
                $top = $procs | Select-Object -First 6
                $topText = ($top | ForEach-Object {
                    "$($_.ProcessName)#$($_.Id) priv=$($_.PrivateMB)MB ws=$($_.WorkingSetMB)MB"
                }) -join " | "
                if ($null -ne $systemMem) {
                    Write-Host "[$stamp] free=$($systemMem.FreeMB)MB used=$($systemMem.UsedMB)MB :: $topText"
                } else {
                    Write-Host "[$stamp] $topText"
                }
            }
        }

        $elapsed = ([DateTimeOffset]::Now - $start).TotalSeconds
        $commandFinished = $null -ne $CommandProcess -and $CommandProcess.HasExited
        $watchOnlyDurationDone = ($null -eq $CommandProcess) -and ($MaxDurationSeconds -gt 0) -and ($elapsed -ge $MaxDurationSeconds)

        if ($commandFinished -or $watchOnlyDurationDone) {
            break
        }

        Start-Sleep -Milliseconds $SampleMs
    }

    if ($minSystemFreeMb -eq [double]::PositiveInfinity) {
        $minSystemFreeMb = $null
    }

    [pscustomobject]@{
        PeakByPid      = $peakByPid
        MaxSystemUsedMB = $maxSystemUsedMb
        MinSystemFreeMB = $minSystemFreeMb
        ElapsedSeconds = [math]::Round((([DateTimeOffset]::Now - $start).TotalSeconds), 3)
    }
}

function Write-IterationSummary {
    param(
        [int]$Iteration,
        [object]$Result,
        [Nullable[int]]$ExitCode,
        [object[]]$Leftovers
    )

    $Leftovers = @($Leftovers)

    Write-Host ""
    Write-Host "=== Iteration $Iteration Summary ==="
    if ($null -ne $ExitCode) {
        Write-Host ("Command exit code: {0}" -f $ExitCode)
    } else {
        Write-Host "Watch-only mode"
    }
    Write-Host ("Elapsed: {0}s" -f $Result.ElapsedSeconds)
    if ($IncludeSystemMemory -and $null -ne $Result.MinSystemFreeMB) {
        Write-Host ("System memory: min free={0}MB, max used={1}MB" -f $Result.MinSystemFreeMB, $Result.MaxSystemUsedMB)
    }

    $peaks = @(
        $Result.PeakByPid.Values | Sort-Object `
            @{ Expression = "PeakPrivateMB"; Descending = $true }, `
            @{ Expression = "ProcessName"; Descending = $false }, `
            @{ Expression = "Id"; Descending = $false }
    )
    if (@($peaks).Count -eq 0) {
        Write-Host "No matching processes observed."
    } else {
        $peaks |
            Select-Object -First 12 ProcessName, Id, PeakPrivateMB, PeakWorkingSetMB, PeakPagedMB, PeakHandles, Samples |
            Format-Table -AutoSize | Out-String | Write-Host
    }

    $leftoverList = @($Leftovers | Where-Object { $null -ne $_ })
    if ($leftoverList.Length -gt 0) {
        Write-Host "Leftover watched processes after iteration (new vs baseline):"
        $leftoverList |
            Select-Object ProcessName, Id, PrivateMB, WorkingSetMB, Handles |
            Format-Table -AutoSize | Out-String | Write-Host
    }
}

if (-not [string]::IsNullOrWhiteSpace($CsvPath)) {
    $csvDir = Split-Path -Parent $CsvPath
    if (-not [string]::IsNullOrWhiteSpace($csvDir) -and -not (Test-Path -LiteralPath $csvDir)) {
        New-Item -ItemType Directory -Path $csvDir -Force | Out-Null
    }
}

Write-Host "Watching patterns: $($ProcessPatterns -join ', ')"
if (-not [string]::IsNullOrWhiteSpace($Executable)) {
    $argPreview = if (@($CommandArgs).Count -gt 0) { " " + (($CommandArgs | ForEach-Object { $_ }) -join " ") } else { "" }
    Write-Host "Command mode: $Executable$argPreview"
    Write-Host "Iterations: $Iterations (sample=${SampleMs}ms, linger=${LingerMs}ms, cooldown=${CooldownMs}ms)"
} else {
    if ($DurationSeconds -gt 0) {
        Write-Host "Watch-only mode for $DurationSeconds seconds (sample=${SampleMs}ms)"
    } else {
        Write-Host "Watch-only mode until Ctrl+C (sample=${SampleMs}ms)"
    }
}
if (-not [string]::IsNullOrWhiteSpace($CsvPath)) {
    Write-Host "CSV logging: $CsvPath"
}

for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
    $baselinePids = @(
        Get-MatchingProcesses -Patterns $ProcessPatterns | ForEach-Object { $_.Id }
    )
    $commandProc = $null
    $exitCode = $null

    if (-not [string]::IsNullOrWhiteSpace($Executable)) {
        Write-Host ""
        Write-Host "Starting iteration $iteration..."
        $commandProc = Start-MonitoredProcess -FilePath $Executable -ProcessArgs $CommandArgs
        Write-Host ("Spawned PID {0}" -f $commandProc.Id)
    } elseif ($iteration -gt 1) {
        break
    }

    $result = Run-MemoryMonitor -Iteration $iteration -CommandProcess $commandProc -MaxDurationSeconds $DurationSeconds

    if ($null -ne $commandProc) {
        if (-not $commandProc.HasExited) {
            try {
                $commandProc.WaitForExit()
            } catch {
                # no-op; we still try to report whatever state is available
            }
        }
        try {
            $exitCode = [int]$commandProc.ExitCode
        } catch {
            $exitCode = $null
        }
    }

    if ($LingerMs -gt 0) {
        Start-Sleep -Milliseconds $LingerMs
    }
    $leftovers = Get-LeftoverWatchProcesses -Patterns $ProcessPatterns -BaselinePids $baselinePids
    Write-IterationSummary -Iteration $iteration -Result $result -ExitCode $exitCode -Leftovers $leftovers

    if ($StopOnFailure -and $null -ne $exitCode -and $exitCode -ne 0) {
        throw "Iteration $iteration failed with exit code $exitCode"
    }
    if ($CooldownMs -gt 0 -and $iteration -lt $Iterations) {
        Start-Sleep -Milliseconds $CooldownMs
    }
}

Write-Host ""
Write-Host "Done."
