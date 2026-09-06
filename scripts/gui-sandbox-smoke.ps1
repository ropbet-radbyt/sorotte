[CmdletBinding(DefaultParameterSetName = "PrepareAndRun")]
param(
    [Parameter(ParameterSetName = "PrepareOnly")]
    [switch]$PrepareOnly,
    [Parameter(Mandatory = $true, ParameterSetName = "RunPrepared")]
    [string]$PreparedRunDirectory,
    [Parameter(ParameterSetName = "PrepareOnly")]
    [Parameter(ParameterSetName = "PrepareAndRun")]
    [ValidateRange(1, 300000)]
    [int]$TimeoutMs = 80000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$artifactRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "target\verification\gui-sandbox-smoke"))
$bundle = Join-Path $PSScriptRoot "gui_sandbox_bundle.py"
$python = (Get-Command python -ErrorAction Stop).Source
. (Join-Path $PSScriptRoot "gui-native-smoke-process.ps1")

if ($PreparedRunDirectory) {
    $runDirectory = (Resolve-Path -LiteralPath $PreparedRunDirectory).Path
    if (-not $runDirectory.StartsWith($artifactRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Prepared runs must be under $artifactRoot"
    }
}
else {
    $stamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
    $runDirectory = Join-Path $artifactRoot "$stamp-$PID"
    $null = New-Item -ItemType Directory -Path $runDirectory
    Write-Host "Preparing Windows Sandbox payload: $runDirectory"
    & $python $bundle source-state --repo-root $repoRoot --run-directory $runDirectory
    if ($LASTEXITCODE -ne 0) { throw "Could not fingerprint native build inputs." }

    $cargo = (Get-Command cargo -ErrorAction Stop).Source
    $builds = [ordered]@{}
    foreach ($name in @("gui", "harness")) {
        $arguments = @("build", "--locked", "-p", "sorotte-gui")
        if ($name -eq "gui") {
            $arguments += @("--bin", "sorotte-gui")
        }
        else {
            $arguments += @("--features", "gui-native-smoke,live-python-interop", "--bin", "sorotte-gui-native-smoke")
        }
        Write-Host "Building $name; output is captured in the run directory."
        $builds[$name] = Invoke-CapturedProcess -FilePath $cargo -Arguments $arguments `
            -WorkingDirectory $repoRoot -ProcessTimeoutMs 600000 `
            -StdoutPath (Join-Path $runDirectory "$name-build-stdout.log") `
            -StderrPath (Join-Path $runDirectory "$name-build-stderr.log")
        $builds | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $runDirectory "builds.json") -Encoding utf8
        if ($builds[$name].exit_code -ne 0) { throw "$name build failed; inspect $runDirectory" }
    }
    $targetDirectory = Join-Path $repoRoot "target"
    if ($env:CARGO_TARGET_DIR) {
        $targetDirectory = if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
            $env:CARGO_TARGET_DIR
        } else { Join-Path $repoRoot $env:CARGO_TARGET_DIR }
    }
    Write-Host "Staging binaries and isolated Python dependencies."
    & $python $bundle prepare --repo-root $repoRoot --run-directory $runDirectory `
        --target-directory $targetDirectory --timeout-ms $TimeoutMs
    if ($LASTEXITCODE -ne 0) { throw "Sandbox preparation failed; inspect $runDirectory" }
}

& $python $bundle validate-payload --run-directory $runDirectory
if ($LASTEXITCODE -ne 0) { throw "Prepared sandbox payload failed validation." }
$configuration = Join-Path $runDirectory "run.wsb"
if ($PrepareOnly) {
    Write-Host "Payload verified. After enabling Windows Sandbox and any required restart, run:"
    Write-Host "powershell -NoProfile -ExecutionPolicy Bypass -File scripts/gui-sandbox-smoke.ps1 -PreparedRunDirectory `"$runDirectory`""
    return
}

$sandbox = Get-Command wsb.exe -ErrorAction SilentlyContinue
if (-not $sandbox) {
    throw "The Windows Sandbox CLI is unavailable. Enable Sandbox, restart if required, and open the Sandbox app once to finish its installation. Prepared payload: $runDirectory"
}
function Invoke-SandboxControl {
    param([string]$Operation, [string[]]$Arguments, [int]$Timeout = 120000)
    $capture = Invoke-CapturedProcess -FilePath $sandbox.Source -Arguments ($Arguments + @("--raw")) `
        -WorkingDirectory $repoRoot -ProcessTimeoutMs $Timeout `
        -StdoutPath (Join-Path $runDirectory "sandbox-$Operation-stdout.log") `
        -StderrPath (Join-Path $runDirectory "sandbox-$Operation-stderr.log")
    if ($capture.exit_code -ne 0) { throw "Sandbox $Operation failed; inspect $runDirectory" }
    return (Get-Content -LiteralPath (Join-Path $runDirectory "sandbox-$Operation-stdout.log") -Raw)
}
function Get-SandboxIds {
    param([string]$Operation)
    $listing = (Invoke-SandboxControl -Operation $Operation -Arguments @("list") -Timeout 15000) | ConvertFrom-Json
    if (-not ($listing.PSObject.Properties.Name -contains "WindowsSandboxEnvironments")) {
        throw "Sandbox CLI returned an unrecognized session inventory."
    }
    return @($listing.WindowsSandboxEnvironments | ForEach-Object { [string]$_.Id })
}
if (@(Get-SandboxIds -Operation "before").Count) {
    throw "An existing Windows Sandbox is running. Close it before starting this disposable test run."
}
$outputDirectory = Join-Path $runDirectory "output"
if (@(Get-ChildItem -LiteralPath $outputDirectory -Force).Count -or (Test-Path -LiteralPath (Join-Path $runDirectory "host-run.json"))) {
    throw "This payload has already been launched. Prepare a fresh run; existing evidence is never overwritten."
}
$manifest = Get-Content -LiteralPath (Join-Path $runDirectory "payload\manifest.json") -Raw | ConvertFrom-Json
$receipt = [ordered]@{
    schema_version = 1
    run_id = $manifest.run_id
    started_at_utc = [DateTime]::UtcNow.ToString("o")
    status = "running"
    sandbox_id = [guid]::NewGuid().ToString()
    sandbox_stopped = $false
    error = $null
}
$receiptPath = Join-Path $runDirectory "host-run.json"
$receipt | ConvertTo-Json | Set-Content -LiteralPath $receiptPath -Encoding utf8
$connector = $null
$startAttempted = $false
try {
    # The caller-chosen UUID identifies only this disposable guest. The modern
    # app's launcher lifetime does not identify the lifetime of its VM.
    $startAttempted = $true
    $null = Invoke-SandboxControl -Operation "start" -Arguments @(
        "start", "--id", $receipt.sandbox_id, "--config", (Get-Content -LiteralPath $configuration -Raw)
    )
    if (@(Get-SandboxIds -Operation "started") -notcontains $receipt.sandbox_id) {
        throw "Sandbox start did not create the requested session."
    }
    $connectArgs = @("connect", "--id", $receipt.sandbox_id, "--raw")
    $connector = Start-Process -FilePath $sandbox.Source `
        -ArgumentList (($connectArgs | ForEach-Object { ConvertTo-ProcessArgument $_ }) -join ' ') `
        -WindowStyle Hidden -PassThru
    Write-Host "Sandbox launched. Keep its viewer open and maximized while the guest tests run."
    $deadline = [DateTime]::UtcNow.AddMilliseconds([long]$manifest.wall_clock_timeout_ms + 240000)
    $completionPath = Join-Path $outputDirectory "completion.json"
    while (-not (Test-Path -LiteralPath $completionPath)) {
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "Sandbox did not produce a completion report before the watchdog deadline."
        }
        Start-Sleep -Milliseconds 500
    }
    & $python $bundle validate-result --run-directory $runDirectory
    if ($LASTEXITCODE -ne 0) { throw "Sandbox native suite failed; inspect $outputDirectory" }
    $receipt.status = "passed"
}
catch {
    $receipt.status = "failed"
    $receipt.error = $_.Exception.Message
    throw
}
finally {
    # Keep diagnostic export independent of the strict suite's exit status and
    # perform it before destroying the isolated desktop.
    try {
        & $python (Join-Path $PSScriptRoot 'native_failure_evidence.py') export `
            --root $outputDirectory --output (Join-Path $runDirectory 'safe-evidence') `
            --source-sha $manifest.source_sha --run-id $manifest.run_id --run-attempt 1 `
            --stage sandbox-native --cleanup pending
        $receipt['evidence_export'] = if ($LASTEXITCODE -eq 0) { 'exported' } else { 'unavailable' }
    }
    catch { $receipt['evidence_export'] = 'unavailable' }
    if ($startAttempted) {
        try {
            if (@(Get-SandboxIds -Operation "before-stop") -contains $receipt.sandbox_id) {
                $null = Invoke-SandboxControl -Operation "stop" -Arguments @("stop", "--id", $receipt.sandbox_id)
            }
            $receipt.sandbox_stopped = @(Get-SandboxIds -Operation "after-stop") -notcontains $receipt.sandbox_id
            if (-not $receipt.sandbox_stopped) {
                throw "Sandbox session remained running after stop."
            }
        }
        catch {
            $receipt.status = "failed"
            $receipt["cleanup_error"] = $_.Exception.Message
        }
    }
    if ($null -ne $connector) { $connector.Dispose() }
    $receipt["finished_at_utc"] = [DateTime]::UtcNow.ToString("o")
    $receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $receiptPath -Encoding utf8
    if (Test-Path -LiteralPath (Join-Path $runDirectory 'safe-evidence') -PathType Container) {
        @{ schema_version=1; kind='sorotte-native-cleanup'; run_id=$manifest.run_id;
           source_sha=$manifest.source_sha; sandbox_stopped=$receipt.sandbox_stopped;
           evidence_export=$receipt.evidence_export } | ConvertTo-Json |
            Set-Content -LiteralPath (Join-Path $runDirectory 'safe-evidence\cleanup.json') -Encoding utf8
    }
    Write-Host "Sandbox artifacts: $runDirectory"
}
if (-not $receipt.sandbox_stopped) { throw "Sandbox cleanup was not confirmed; inspect $receiptPath" }
