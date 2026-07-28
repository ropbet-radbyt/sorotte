param(
    [switch]$Json,
    [switch]$KeepOpen,
    [string]$BinaryPath,
    [int]$TimeoutMs = 80000,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ExtraArgs = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($KeepOpen) {
    throw "-KeepOpen is exploratory and cannot produce strict native-smoke evidence"
}

if ($TimeoutMs -le 0) {
    throw "-TimeoutMs must be greater than zero"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$validatorPath = Join-Path $PSScriptRoot "gui_native_smoke_contract.py"
$processHelperPath = Join-Path $PSScriptRoot "gui-native-smoke-process.ps1"
. $processHelperPath
$pythonCommand = Get-Command python -ErrorAction Stop

$requestedScenarios = [System.Collections.Generic.List[string]]::new()
$index = 0
while ($index -lt $ExtraArgs.Length) {
    if ($ExtraArgs[$index] -ne "--scenario") {
        throw "strict native smoke accepts only repeated --scenario NAME arguments; got '$($ExtraArgs[$index])'"
    }
    if ($index + 1 -ge $ExtraArgs.Length) {
        throw "--scenario requires a scenario name"
    }
    $requestedScenarios.Add($ExtraArgs[$index + 1])
    $index += 2
}

if ($requestedScenarios.Count -eq 0) {
    $defaultScenarios = @(& $pythonCommand.Source $validatorPath --print-default-scenarios)
    if ($LASTEXITCODE -ne 0 -or $defaultScenarios.Count -eq 0) {
        throw "failed to load the required native-smoke scenario inventory"
    }
    foreach ($scenario in $defaultScenarios) {
        $requestedScenarios.Add($scenario)
    }
}

$scenarioCheckArgs = @($validatorPath, "--check-scenarios")
foreach ($scenario in $requestedScenarios) {
    $scenarioCheckArgs += "--scenario"
    $scenarioCheckArgs += $scenario
}
& $pythonCommand.Source @scenarioCheckArgs
if ($LASTEXITCODE -ne 0) {
    throw "required native-smoke scenario selection is invalid"
}

$targetRoot = if ($env:CARGO_TARGET_DIR) {
    if ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR
    }
    else {
        Join-Path $repoRoot $env:CARGO_TARGET_DIR
    }
}
else {
    Join-Path $repoRoot "target"
}
$effectiveBinaryPath = if ($BinaryPath) {
    if ([System.IO.Path]::IsPathRooted($BinaryPath)) {
        [System.IO.Path]::GetFullPath($BinaryPath)
    }
    else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $BinaryPath))
    }
}
else {
    Join-Path $targetRoot "debug\sorotte-gui.exe"
}
$binaryProvenance = if ($BinaryPath) { "caller-supplied" } else { "rebuilt-debug" }

$suiteArgs = @("--json", "--binary", $effectiveBinaryPath)
$suiteArgs += "--timeout-ms"
$suiteArgs += [string]$TimeoutMs
foreach ($scenario in $requestedScenarios) {
    $suiteArgs += "--scenario"
    $suiteArgs += $scenario
}

$nativeSmokeFeatures = "gui-native-smoke,live-python-interop"
$nativeHarnessPath = Join-Path $targetRoot "debug\sorotte-gui-native-smoke.exe"

$timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
$artifactDirectory = Join-Path $repoRoot "target\verification\gui-native-smoke\$timestamp-$PID"
$null = New-Item -ItemType Directory -Force -Path $artifactDirectory
$reportPath = Join-Path $artifactDirectory "native-report.json"
$stderrPath = Join-Path $artifactDirectory "native-stderr.log"
$summaryPath = Join-Path $artifactDirectory "contract-summary.json"
$metadataPath = Join-Path $artifactDirectory "invocation.json"
$buildStdoutPath = Join-Path $artifactDirectory "build-stdout.log"
$buildStderrPath = Join-Path $artifactDirectory "build-stderr.log"
$harnessBuildStdoutPath = Join-Path $artifactDirectory "harness-build-stdout.log"
$harnessBuildStderrPath = Join-Path $artifactDirectory "harness-build-stderr.log"
$timeoutGraceMs = 30000
$startedAtUtc = [DateTime]::UtcNow.ToString("o")
$nativeWallClockTimeout = ([long]$TimeoutMs * ([long]$requestedScenarios.Count + 1)) + $timeoutGraceMs
if ($nativeWallClockTimeout -gt [int]::MaxValue) {
    throw "derived native-smoke wall-clock timeout exceeds the supported process wait"
}
$nativeWallClockTimeoutMs = [int]$nativeWallClockTimeout
$buildWallClockTimeoutMs = 600000

[pscustomobject]@{
    schema_version = 1
    kind = "sorotte-gui-native-smoke-invocation"
    started_at_utc = $startedAtUtc
    timeout_ms = $TimeoutMs
    required_scenarios = @($requestedScenarios)
    binary_path = $effectiveBinaryPath
    binary_provenance = $binaryProvenance
    runner_timeout_ms = $TimeoutMs
    wall_clock_timeout_ms = $nativeWallClockTimeoutMs
    timeout_grace_ms = $timeoutGraceMs
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $metadataPath -Encoding utf8

$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$prelaunchError = $null
if (-not $cargoCommand) {
    $prelaunchError = "cargo executable was not found"
}
$callerBinaryExists = Test-Path -LiteralPath $effectiveBinaryPath -PathType Leaf
if ($BinaryPath -and -not $callerBinaryExists) {
    $prelaunchError = "caller-supplied native-smoke binary does not exist: $effectiveBinaryPath"
}

$buildExitCode = 0
$buildResult = [pscustomobject]@{
    exit_code = 0
    timed_out = $false
    duration_ms = 0
    process_timeout_ms = $null
    tree_kill_exit_code = $null
    start_error = $null
}
$harnessBuildResult = $null
if (-not $BinaryPath -and -not $prelaunchError) {
    $buildArgs = @(
        "build",
        "--quiet",
        "--locked",
        "-p",
        "sorotte-gui",
        "--bin",
        "sorotte-gui"
    )
    $buildResult = Invoke-CapturedProcess `
        -FilePath $cargoCommand.Source `
        -Arguments $buildArgs `
        -WorkingDirectory $repoRoot `
        -StdoutPath $buildStdoutPath `
        -StderrPath $buildStderrPath `
        -ProcessTimeoutMs $buildWallClockTimeoutMs
    $buildExitCode = $buildResult.exit_code
}
else {
    [System.IO.File]::WriteAllText($buildStdoutPath, "", $utf8WithoutBom)
    [System.IO.File]::WriteAllText($buildStderrPath, "", $utf8WithoutBom)
}

if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    $harnessBuildArgs = @(
        "build",
        "--quiet",
        "--locked",
        "-p",
        "sorotte-gui",
        "--features",
        $nativeSmokeFeatures,
        "--bin",
        "sorotte-gui-native-smoke"
    )
    $harnessBuildResult = Invoke-CapturedProcess `
        -FilePath $cargoCommand.Source `
        -Arguments $harnessBuildArgs `
        -WorkingDirectory $repoRoot `
        -StdoutPath $harnessBuildStdoutPath `
        -StderrPath $harnessBuildStderrPath `
        -ProcessTimeoutMs $buildWallClockTimeoutMs
    $buildExitCode = $harnessBuildResult.exit_code
}
else {
    [System.IO.File]::WriteAllText($harnessBuildStdoutPath, "", $utf8WithoutBom)
    [System.IO.File]::WriteAllText($harnessBuildStderrPath, "", $utf8WithoutBom)
}

if ($buildExitCode -eq 0 -and -not $prelaunchError -and -not (
    Test-Path -LiteralPath $nativeHarnessPath -PathType Leaf
)) {
    $prelaunchError = "prebuilt native-smoke harness does not exist: $nativeHarnessPath"
}

$binarySha256Before = $null
if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    try {
        $binarySha256Before = (
            Get-FileHash -LiteralPath $effectiveBinaryPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
    }
    catch {
        $prelaunchError = "failed to hash native GUI before launch: $($_.Exception.Message)"
    }
}

$nativeExitCode = $buildExitCode
$nativeResult = $null
if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    $nativeResult = Invoke-CapturedProcess `
        -FilePath $nativeHarnessPath `
        -Arguments $suiteArgs `
        -WorkingDirectory $repoRoot `
        -StdoutPath $reportPath `
        -StderrPath $stderrPath `
        -ProcessTimeoutMs $nativeWallClockTimeoutMs
    $nativeExitCode = $nativeResult.exit_code
}
else {
    if ($prelaunchError) {
        $nativeExitCode = 125
    }
    $buildErrorReport = [pscustomobject]@{
        result = "error"
        error = if ($prelaunchError) {
            $prelaunchError
        }
        else {
            "fresh native GUI or harness build failed with exit code $buildExitCode"
        }
    } | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($reportPath, $buildErrorReport, $utf8WithoutBom)
    $failureStderr = if ($prelaunchError) {
        "HARNESS_PRELAUNCH_FAILURE: $prelaunchError`n"
    }
    elseif ($buildResult.exit_code -ne 0) {
        [System.IO.File]::ReadAllText($buildStderrPath)
    }
    else {
        [System.IO.File]::ReadAllText($harnessBuildStderrPath)
    }
    [System.IO.File]::WriteAllText($stderrPath, $failureStderr, $utf8WithoutBom)
}

$binarySha256After = $null
if ($binarySha256Before) {
    try {
        $binarySha256After = (
            Get-FileHash -LiteralPath $effectiveBinaryPath -Algorithm SHA256
        ).Hash.ToLowerInvariant()
        if ($binarySha256After -ne $binarySha256Before) {
            throw "native GUI SHA-256 changed during execution"
        }
    }
    catch {
        $hashError = "HARNESS_BINARY_PROVENANCE_FAILURE: $($_.Exception.Message)"
        [System.IO.File]::AppendAllText($stderrPath, "`n$hashError`n", $utf8WithoutBom)
        if ($nativeExitCode -eq 0) {
            $nativeExitCode = 126
        }
    }
}
[pscustomobject]@{
    schema_version = 1
    kind = "sorotte-gui-native-smoke-invocation"
    started_at_utc = $startedAtUtc
    finished_at_utc = [DateTime]::UtcNow.ToString("o")
    timeout_ms = $TimeoutMs
    required_scenarios = @($requestedScenarios)
    binary_path = $effectiveBinaryPath
    binary_sha256_before = $binarySha256Before
    binary_sha256_after = $binarySha256After
    binary_provenance = $binaryProvenance
    build = $buildResult
    harness_build = $harnessBuildResult
    runner = $nativeResult
    runner_timeout_ms = $TimeoutMs
    wall_clock_timeout_ms = $nativeWallClockTimeoutMs
    timeout_grace_ms = $timeoutGraceMs
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $metadataPath -Encoding utf8

$expectedBinaryShaArgument = if ($binarySha256Before) {
    $binarySha256Before
}
else {
    "unavailable"
}
$validatorArgs = @(
    $validatorPath,
    "--report", $reportPath,
    "--stderr", $stderrPath,
    "--summary", $summaryPath,
    "--expected-binary", $effectiveBinaryPath,
    "--expected-binary-sha256", $expectedBinaryShaArgument,
    "--producer-exit-code", [string]$nativeExitCode
)
foreach ($scenario in $requestedScenarios) {
    $validatorArgs += "--scenario"
    $validatorArgs += $scenario
}

& $pythonCommand.Source @validatorArgs
$validatorExitCode = $LASTEXITCODE

Get-Content -LiteralPath $reportPath
if ($nativeExitCode -ne 0) {
    Write-Error "native smoke build or runner exited $nativeExitCode; artifacts: $artifactDirectory" -ErrorAction Continue
    if (Test-Path -LiteralPath $stderrPath) {
        Get-Content -LiteralPath $stderrPath | ForEach-Object { [Console]::Error.WriteLine($_) }
    }
    exit $nativeExitCode
}

if ($validatorExitCode -ne 0) {
    Write-Error "native smoke report failed its strict contract; artifacts: $artifactDirectory" -ErrorAction Continue
    exit $validatorExitCode
}

Write-Verbose "Strict native-smoke artifacts: $artifactDirectory"
exit 0
