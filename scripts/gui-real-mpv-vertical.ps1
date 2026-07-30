param(
    [string]$MpvPath,
    [string]$BinaryPath,
    [int]$TimeoutMs = 30000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($TimeoutMs -le 0) {
    throw "-TimeoutMs must be greater than zero"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$processHelperPath = Join-Path $PSScriptRoot "gui-native-smoke-process.ps1"
$validatorPath = Join-Path $PSScriptRoot "gui_real_mpv_vertical_contract.py"
. $processHelperPath

$targetRoot = if ($env:CARGO_TARGET_DIR) {
    if ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    }
    else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $env:CARGO_TARGET_DIR))
    }
}
else {
    Join-Path $repoRoot "target"
}
$timestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssfffZ")
$artifactDirectory = Join-Path $targetRoot "verification\gui-real-mpv-vertical\$timestamp-$PID"
if (Test-Path -LiteralPath $artifactDirectory) {
    throw "fresh real-mpv artifact directory already exists: $artifactDirectory"
}
$null = New-Item -ItemType Directory -Path $artifactDirectory

$reportPath = Join-Path $artifactDirectory "harness-report.json"
$stderrPath = Join-Path $artifactDirectory "harness-stderr.log"
$summaryPath = Join-Path $artifactDirectory "contract-summary.json"
$metadataPath = Join-Path $artifactDirectory "invocation.json"
$guiBuildStdoutPath = Join-Path $artifactDirectory "gui-build-stdout.log"
$guiBuildStderrPath = Join-Path $artifactDirectory "gui-build-stderr.log"
$harnessBuildStdoutPath = Join-Path $artifactDirectory "harness-build-stdout.log"
$harnessBuildStderrPath = Join-Path $artifactDirectory "harness-build-stderr.log"
$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
$startedAtUtc = [DateTime]::UtcNow.ToString("o")

function Write-InvocationMetadata {
    param(
        [Parameter(Mandatory = $true)][string]$Result,
        [AllowNull()]$GuiBuild,
        [AllowNull()]$HarnessBuild,
        [AllowNull()]$Runner,
        [AllowNull()][string]$EffectiveGuiPath,
        [AllowNull()][string]$GuiSha256Before,
        [AllowNull()][string]$GuiSha256After,
        [AllowNull()][string]$EffectiveMpvPath,
        [AllowNull()][string]$MpvSha256,
        [AllowNull()][string]$ErrorMessage
    )

    [pscustomobject]@{
        schema_version = 1
        kind = "sorotte-gui-real-mpv-vertical-invocation"
        result = $Result
        started_at_utc = $startedAtUtc
        finished_at_utc = if ($Result -eq "running") { $null } else { [DateTime]::UtcNow.ToString("o") }
        timeout_ms = $TimeoutMs
        artifact_directory = $artifactDirectory
        gui_binary_path = $EffectiveGuiPath
        gui_binary_sha256_before = $GuiSha256Before
        gui_binary_sha256_after = $GuiSha256After
        gui_binary_provenance = if ($BinaryPath) { "caller-supplied" } else { "rebuilt-debug" }
        mpv_binary_path = $EffectiveMpvPath
        mpv_binary_sha256 = $MpvSha256
        gui_build = $GuiBuild
        harness_build = $HarnessBuild
        runner = $Runner
        error = $ErrorMessage
    } | ConvertTo-Json -Depth 7 | Set-Content -LiteralPath $metadataPath -Encoding utf8
}

Write-InvocationMetadata `
    -Result "running" `
    -GuiBuild $null `
    -HarnessBuild $null `
    -Runner $null `
    -EffectiveGuiPath $null `
    -GuiSha256Before $null `
    -GuiSha256After $null `
    -EffectiveMpvPath $null `
    -MpvSha256 $null `
    -ErrorMessage $null

$mpvCandidates = [System.Collections.Generic.List[string]]::new()
if ($MpvPath) {
    $mpvCandidates.Add($MpvPath)
}
elseif ($env:SOROTTE_REAL_MPV_BIN) {
    $mpvCandidates.Add($env:SOROTTE_REAL_MPV_BIN)
}
else {
    $pathMpv = Get-Command mpv -ErrorAction SilentlyContinue
    if ($pathMpv) {
        $mpvCandidates.Add($pathMpv.Source)
    }
    $mpvCandidates.Add((Join-Path $repoRoot "mpv\mpv.exe"))
    if ($env:ProgramFiles) {
        $mpvCandidates.Add((Join-Path $env:ProgramFiles "mpv\mpv.exe"))
    }
    if (${env:ProgramFiles(x86)}) {
        $mpvCandidates.Add((Join-Path ${env:ProgramFiles(x86)} "mpv\mpv.exe"))
    }
    if ($env:LOCALAPPDATA) {
        $mpvCandidates.Add((Join-Path $env:LOCALAPPDATA "Programs\mpv\mpv.exe"))
    }
}

$effectiveMpvPath = $null
foreach ($candidate in $mpvCandidates) {
    $absoluteCandidate = if ([System.IO.Path]::IsPathRooted($candidate)) {
        [System.IO.Path]::GetFullPath($candidate)
    }
    else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $candidate))
    }
    if (Test-Path -LiteralPath $absoluteCandidate -PathType Leaf) {
        $effectiveMpvPath = (Resolve-Path -LiteralPath $absoluteCandidate).Path
        break
    }
}

if (-not $effectiveMpvPath) {
    $preflightError = if ($MpvPath) {
        "required mpv binary does not exist: $MpvPath"
    }
    else {
        "no supported mpv candidate was found; pass -MpvPath or set SOROTTE_REAL_MPV_BIN"
    }
    $failureReport = [pscustomobject]@{
        schema_version = 1
        kind = "sorotte-gui-real-mpv-vertical"
        result = "error"
        capability = "missing-prerequisite"
        stage = "mpv-preflight"
        error = $preflightError
        artifact_root = $artifactDirectory
    } | ConvertTo-Json -Depth 4 -Compress
    [System.IO.File]::WriteAllText($reportPath, $failureReport, $utf8WithoutBom)
    [System.IO.File]::WriteAllText(
        $stderrPath,
        "HARNESS_PRELAUNCH_FAILURE: $preflightError`n",
        $utf8WithoutBom
    )
    foreach ($path in @(
        $guiBuildStdoutPath,
        $guiBuildStderrPath,
        $harnessBuildStdoutPath,
        $harnessBuildStderrPath
    )) {
        [System.IO.File]::WriteAllText($path, "", $utf8WithoutBom)
    }
    Write-InvocationMetadata `
        -Result "error" `
        -GuiBuild $null `
        -HarnessBuild $null `
        -Runner $null `
        -EffectiveGuiPath $null `
        -GuiSha256Before $null `
        -GuiSha256After $null `
        -EffectiveMpvPath $null `
        -MpvSha256 $null `
        -ErrorMessage $preflightError
    Get-Content -LiteralPath $reportPath
    Write-Error "real-mpv preflight failed; artifacts: $artifactDirectory" -ErrorAction Continue
    exit 125
}

$effectiveGuiPath = if ($BinaryPath) {
    $candidate = if ([System.IO.Path]::IsPathRooted($BinaryPath)) {
        [System.IO.Path]::GetFullPath($BinaryPath)
    }
    else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $BinaryPath))
    }
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        (Resolve-Path -LiteralPath $candidate).Path
    }
    else {
        $candidate
    }
}
else {
    Join-Path $targetRoot "debug\sorotte-gui.exe"
}
$nativeHarnessPath = Join-Path $targetRoot "debug\sorotte-gui-native-smoke.exe"
$mpvSha256 = (
    Get-FileHash -LiteralPath $effectiveMpvPath -Algorithm SHA256
).Hash.ToLowerInvariant()

$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$pythonCommand = Get-Command python -ErrorAction SilentlyContinue
$prelaunchError = $null
if (-not $cargoCommand) {
    $prelaunchError = "cargo executable was not found"
}
elseif (-not $pythonCommand) {
    $prelaunchError = "python executable was not found"
}
elseif ($BinaryPath -and -not (Test-Path -LiteralPath $effectiveGuiPath -PathType Leaf)) {
    $prelaunchError = "caller-supplied GUI binary does not exist: $effectiveGuiPath"
}

$buildTimeoutMs = 600000
$guiBuild = $null
$harnessBuild = $null
$buildExitCode = 0
if (-not $BinaryPath -and -not $prelaunchError) {
    $guiBuild = Invoke-CapturedProcess `
        -FilePath $cargoCommand.Source `
        -Arguments @("build", "--quiet", "--locked", "-p", "sorotte-gui", "--bin", "sorotte-gui") `
        -WorkingDirectory $repoRoot `
        -StdoutPath $guiBuildStdoutPath `
        -StderrPath $guiBuildStderrPath `
        -ProcessTimeoutMs $buildTimeoutMs
    $buildExitCode = $guiBuild.exit_code
}
else {
    [System.IO.File]::WriteAllText($guiBuildStdoutPath, "", $utf8WithoutBom)
    [System.IO.File]::WriteAllText($guiBuildStderrPath, "", $utf8WithoutBom)
}

if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    $harnessBuild = Invoke-CapturedProcess `
        -FilePath $cargoCommand.Source `
        -Arguments @(
            "build",
            "--quiet",
            "--locked",
            "-p",
            "sorotte-gui",
            "--features",
            "gui-native-smoke",
            "--bin",
            "sorotte-gui-native-smoke"
        ) `
        -WorkingDirectory $repoRoot `
        -StdoutPath $harnessBuildStdoutPath `
        -StderrPath $harnessBuildStderrPath `
        -ProcessTimeoutMs $buildTimeoutMs
    $buildExitCode = $harnessBuild.exit_code
}
else {
    [System.IO.File]::WriteAllText($harnessBuildStdoutPath, "", $utf8WithoutBom)
    [System.IO.File]::WriteAllText($harnessBuildStderrPath, "", $utf8WithoutBom)
}

if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    if (-not (Test-Path -LiteralPath $effectiveGuiPath -PathType Leaf)) {
        $prelaunchError = "fresh GUI binary does not exist: $effectiveGuiPath"
    }
    elseif (-not (Test-Path -LiteralPath $nativeHarnessPath -PathType Leaf)) {
        $prelaunchError = "fresh native harness does not exist: $nativeHarnessPath"
    }
}

$guiSha256Before = $null
if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    $guiSha256Before = (
        Get-FileHash -LiteralPath $effectiveGuiPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
}

$runner = $null
$producerExitCode = $buildExitCode
if ($buildExitCode -eq 0 -and -not $prelaunchError) {
    $runnerTimeout = ([long]$TimeoutMs * 8) + 30000
    if ($runnerTimeout -gt [int]::MaxValue) {
        throw "derived real-mpv wall-clock timeout exceeds supported process wait"
    }
    $runner = Invoke-CapturedProcess `
        -FilePath $nativeHarnessPath `
        -Arguments @(
            "--real-mpv-vertical",
            "--json",
            "--binary",
            $effectiveGuiPath,
            "--mpv",
            $effectiveMpvPath,
            "--artifact-dir",
            $artifactDirectory,
            "--timeout-ms",
            [string]$TimeoutMs
        ) `
        -WorkingDirectory $repoRoot `
        -StdoutPath $reportPath `
        -StderrPath $stderrPath `
        -ProcessTimeoutMs ([int]$runnerTimeout) `
        -EnvironmentVariables @{
            SOROTTE_GUI_NATIVE_SMOKE_ARTIFACT_DIR = $artifactDirectory
        }
    $producerExitCode = $runner.exit_code
}
else {
    $producerExitCode = if ($prelaunchError) { 125 } else { $buildExitCode }
    $failure = if ($prelaunchError) {
        $prelaunchError
    }
    else {
        "fresh GUI or native harness build failed with exit code $buildExitCode"
    }
    [System.IO.File]::WriteAllText(
        $reportPath,
        ([pscustomobject]@{
            schema_version = 1
            kind = "sorotte-gui-real-mpv-vertical"
            result = "error"
            capability = "not-executed"
            stage = "build"
            error = $failure
        } | ConvertTo-Json -Compress),
        $utf8WithoutBom
    )
    [System.IO.File]::WriteAllText(
        $stderrPath,
        "HARNESS_PRELAUNCH_FAILURE: $failure`n",
        $utf8WithoutBom
    )
}

$guiSha256After = $null
if ($guiSha256Before) {
    $guiSha256After = (
        Get-FileHash -LiteralPath $effectiveGuiPath -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    if ($guiSha256After -ne $guiSha256Before -and $producerExitCode -eq 0) {
        [System.IO.File]::AppendAllText(
            $stderrPath,
            "`nHARNESS_BINARY_PROVENANCE_FAILURE: GUI digest changed during execution`n",
            $utf8WithoutBom
        )
        $producerExitCode = 126
    }
}

$validatorExitCode = 1
if ($pythonCommand -and $guiSha256Before) {
    & $pythonCommand.Source `
        $validatorPath `
        --report $reportPath `
        --artifact-dir $artifactDirectory `
        --expected-gui $effectiveGuiPath `
        --expected-gui-sha256 $guiSha256Before `
        --expected-mpv $effectiveMpvPath `
        --expected-mpv-sha256 $mpvSha256 `
        --producer-exit-code $producerExitCode `
        --summary $summaryPath
    $validatorExitCode = $LASTEXITCODE
}
else {
    [System.IO.File]::WriteAllText(
        $summaryPath,
        ([pscustomobject]@{
            schema_version = 1
            kind = "sorotte-gui-real-mpv-vertical-contract"
            result = "error"
            error = "validator prerequisites were unavailable"
        } | ConvertTo-Json -Compress),
        $utf8WithoutBom
    )
}

$finalResult = if ($producerExitCode -eq 0 -and $validatorExitCode -eq 0) {
    "passed"
}
else {
    "error"
}
Write-InvocationMetadata `
    -Result $finalResult `
    -GuiBuild $guiBuild `
    -HarnessBuild $harnessBuild `
    -Runner $runner `
    -EffectiveGuiPath $effectiveGuiPath `
    -GuiSha256Before $guiSha256Before `
    -GuiSha256After $guiSha256After `
    -EffectiveMpvPath $effectiveMpvPath `
    -MpvSha256 $mpvSha256 `
    -ErrorMessage $(if ($finalResult -eq "passed") { $null } else { "producer=$producerExitCode validator=$validatorExitCode" })

Get-Content -LiteralPath $reportPath
if ($producerExitCode -ne 0) {
    Write-Error "real-mpv harness exited $producerExitCode; artifacts: $artifactDirectory" -ErrorAction Continue
    if (Test-Path -LiteralPath $stderrPath) {
        Get-Content -LiteralPath $stderrPath | ForEach-Object {
            [Console]::Error.WriteLine($_)
        }
    }
    exit $producerExitCode
}
if ($validatorExitCode -ne 0) {
    Write-Error "real-mpv report failed its strict contract; artifacts: $artifactDirectory" -ErrorAction Continue
    exit $validatorExitCode
}

Write-Verbose "Real-mpv vertical artifacts: $artifactDirectory"
exit 0
