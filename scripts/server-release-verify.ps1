param(
    [switch]$NoWorkspace,
    [ValidateSet("All", "Prepare", "Behavior")]
    [string]$Stage = "All",
    [string]$WorkspaceReceipt = "",
    [string]$ReceiptRunId = $env:GITHUB_RUN_ID,
    [string]$ReportDir = "target/server-release-verify"
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
Set-Location $RepoRoot

$LegacyRepo = "https://github.com/Syncplay/syncplay.git"
$LegacyRef = "v1.7.5"
$LegacySha = (& python -c "import sys; sys.path.insert(0, 'scripts'); import verification_tools; print(verification_tools.pins()['references']['legacy-sha'])").Trim()
if ($LASTEXITCODE -ne 0) { throw "Cannot load immutable legacy reference pin" }
$StartedAtUtc = (Get-Date).ToUniversalTime()
$Results = New-Object System.Collections.Generic.List[object]
$Failure = $null
$LegacyOraclePath = $null
$LegacyOracleSource = $null

function Resolve-ReleasePath {
    param([Parameter(Mandatory = $true)][string]$Path)

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return [System.IO.Path]::GetFullPath($Path)
    }
    return [System.IO.Path]::GetFullPath((Join-Path $RepoRoot $Path))
}

function Assert-PathInsideRepo {
    param([Parameter(Mandatory = $true)][string]$Path)

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    $repoPath = [System.IO.Path]::GetFullPath($RepoRoot)
    $separator = [System.IO.Path]::DirectorySeparatorChar
    $repoPrefix = if ($repoPath.EndsWith([string]$separator)) {
        $repoPath
    } else {
        "$repoPath$separator"
    }
    $comparison = if ($separator -eq '\') {
        [System.StringComparison]::OrdinalIgnoreCase
    } else {
        [System.StringComparison]::Ordinal
    }
    if (
        -not $fullPath.Equals($repoPath, $comparison) -and
        -not $fullPath.StartsWith($repoPrefix, $comparison)
    ) {
        throw "Refusing to mutate path outside repo: $fullPath"
    }
}

function Add-ServerReleaseResult {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string]$Status,
        [Parameter(Mandatory = $true)][double]$Seconds,
        [string]$Details = ""
    )

    $Results.Add([pscustomobject]@{
        Step = $Name
        Command = $Command
        Status = $Status
        Seconds = [Math]::Round($Seconds, 1)
        Details = $Details
    })
}

function Invoke-ServerReleaseCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @(),
        [hashtable]$Environment = @{}
    )

    Write-Host ""
    Write-Host "==> $Name" -ForegroundColor Cyan
    Write-Host "$FilePath $($Arguments -join ' ')"

    $backup = @{}
    foreach ($key in $Environment.Keys) {
        $backup[$key] = [Environment]::GetEnvironmentVariable($key, "Process")
        [Environment]::SetEnvironmentVariable($key, [string]$Environment[$key], "Process")
    }

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $errorText = ""
    try {
        & $FilePath @Arguments
        $exitCode = $LASTEXITCODE
    }
    catch {
        $exitCode = 1
        $errorText = $_.Exception.Message
    }
    finally {
        $watch.Stop()
        foreach ($key in $Environment.Keys) {
            [Environment]::SetEnvironmentVariable($key, $backup[$key], "Process")
        }
    }

    $status = if ($exitCode -eq 0) { "PASS" } else { "FAIL" }
    Add-ServerReleaseResult $Name "$FilePath $($Arguments -join ' ')" $status $watch.Elapsed.TotalSeconds $errorText
    if ($exitCode -ne 0) {
        if ([string]::IsNullOrWhiteSpace($errorText)) {
            $errorText = "exit code $exitCode"
        }
        throw "Step '$Name' failed: $errorText"
    }
}

function Invoke-CargoStep {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$CargoArgs,
        [hashtable]$Environment = @{}
    )

    Invoke-ServerReleaseCommand $Name "cargo" $CargoArgs $Environment
}

function Get-ToolOutput {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [string[]]$Arguments = @()
    )

    try {
        $output = & $FilePath @Arguments 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) {
            return "$FilePath failed with exit code $LASTEXITCODE"
        }
        return $output.Trim()
    }
    catch {
        return $_.Exception.Message
    }
}

function Test-LegacyOracleReady {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath (Join-Path $Path "syncplayServer.py") -PathType Leaf)) { return $false }
    & python scripts/release_qualification.py verify-legacy --legacy-root $Path
    if ($LASTEXITCODE -ne 0) { throw "Legacy oracle must be clean and pinned to $LegacySha" }
    return $true
}

function Ensure-LegacyOracle {
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $configured = [Environment]::GetEnvironmentVariable("SYNCPLAY_LEGACY_ROOT", "Process")
    if (-not [string]::IsNullOrWhiteSpace($configured)) {
        $configuredPath = [System.IO.Path]::GetFullPath($configured)
        if (-not (Test-LegacyOracleReady $configuredPath)) {
            $watch.Stop()
            Add-ServerReleaseResult "legacy oracle" "SYNCPLAY_LEGACY_ROOT=$configuredPath" "FAIL" $watch.Elapsed.TotalSeconds "syncplayServer.py not found"
            throw "Strict server release verification requires a usable SYNCPLAY_LEGACY_ROOT: $configuredPath"
        }
        $watch.Stop()
        Add-ServerReleaseResult "legacy oracle" "SYNCPLAY_LEGACY_ROOT=$configuredPath" "PASS" $watch.Elapsed.TotalSeconds "configured checkout"
        return [pscustomobject]@{ Path = $configuredPath; Source = "SYNCPLAY_LEGACY_ROOT" }
    }

    $repoLocal = Resolve-ReleasePath ".interop-cache/syncplay-legacy"
    if (Test-LegacyOracleReady $repoLocal) {
        [Environment]::SetEnvironmentVariable("SYNCPLAY_LEGACY_ROOT", $repoLocal, "Process")
        $watch.Stop()
        Add-ServerReleaseResult "legacy oracle" "reuse $repoLocal" "PASS" $watch.Elapsed.TotalSeconds "repo-local checkout"
        return [pscustomobject]@{ Path = $repoLocal; Source = "repo-local" }
    }

    Write-Host ""
    Write-Host "==> legacy oracle" -ForegroundColor Cyan
    Write-Host "git clone --depth 1 --branch $LegacyRef --single-branch $LegacyRepo $repoLocal"

    Assert-PathInsideRepo $repoLocal
    if (Test-Path -LiteralPath $repoLocal) {
        Remove-Item -LiteralPath $repoLocal -Recurse -Force
    }
    $parent = Split-Path -Parent $repoLocal
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    & git clone --depth 1 --branch $LegacyRef --single-branch $LegacyRepo $repoLocal
    $cloneExit = $LASTEXITCODE
    if ($cloneExit -ne 0 -or -not (Test-LegacyOracleReady $repoLocal)) {
        $watch.Stop()
        Add-ServerReleaseResult "legacy oracle" "git clone $LegacyRepo $repoLocal" "FAIL" $watch.Elapsed.TotalSeconds "clone failed"
        throw "Failed to bootstrap Syncplay $LegacyRef oracle at $repoLocal"
    }
    [Environment]::SetEnvironmentVariable("SYNCPLAY_LEGACY_ROOT", $repoLocal, "Process")
    $watch.Stop()
    Add-ServerReleaseResult "legacy oracle" "git clone $LegacyRepo $repoLocal" "PASS" $watch.Elapsed.TotalSeconds "bootstrapped $LegacyRef"
    return [pscustomobject]@{ Path = $repoLocal; Source = "bootstrapped" }
}

function Escape-MarkdownCell {
    param([string]$Value)
    if ($null -eq $Value) {
        return ""
    }
    return $Value.Replace("|", "\|").Replace("`r", " ").Replace("`n", " ")
}

function Write-ServerReleaseReports {
    $reportRoot = Resolve-ReleasePath $ReportDir
    New-Item -ItemType Directory -Force -Path $reportRoot | Out-Null

    $failedSteps = @($Results | Where-Object { $_.Status -ne "PASS" })
    $status = if ($failedSteps.Count -eq 0 -and $null -eq $Failure) { "PASS" } else { "FAIL" }
    $finishedAtUtc = (Get-Date).ToUniversalTime()
    $pythonBin = [Environment]::GetEnvironmentVariable("SYNCPLAY_PYTHON_BIN", "Process")
    if ([string]::IsNullOrWhiteSpace($pythonBin)) {
        $pythonBin = "python"
    }

    $report = [pscustomobject]@{
        status = $status
        startedAtUtc = $StartedAtUtc.ToString("o")
        finishedAtUtc = $finishedAtUtc.ToString("o")
        elapsedSeconds = [Math]::Round(($finishedAtUtc - $StartedAtUtc).TotalSeconds, 1)
        repoRoot = $RepoRoot
        reportDir = $reportRoot
        noWorkspace = [bool]$NoWorkspace
        stage = $Stage
        workspaceReceipt = $WorkspaceReceipt
        sourceSha = (Get-ToolOutput -FilePath "git" -Arguments @("rev-parse", "HEAD"))
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
        rustc = (Get-ToolOutput -FilePath "rustc" -Arguments @("-Vv"))
        cargo = (Get-ToolOutput -FilePath "cargo" -Arguments @("-V"))
        python = [pscustomobject]@{
            bin = $pythonBin
            version = (Get-ToolOutput -FilePath $pythonBin -Arguments @("--version"))
        }
        legacyOracle = [pscustomobject]@{
            ref = $LegacyRef
            sha = $LegacySha
            repository = $LegacyRepo
            path = $LegacyOraclePath
            source = $LegacyOracleSource
        }
        steps = $Results.ToArray()
    }

    $jsonPath = Join-Path $reportRoot "server-release-report.json"
    $mdPath = Join-Path $reportRoot "server-release-report.md"
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $jsonPath -Encoding UTF8

    $lines = New-Object System.Collections.Generic.List[string]
    $lines.Add("# Sorotte Server Release Verification")
    $lines.Add("")
    $lines.Add("- Status: $status")
    $lines.Add("- Started UTC: $($report.startedAtUtc)")
    $lines.Add("- Finished UTC: $($report.finishedAtUtc)")
    $lines.Add("- Elapsed seconds: $($report.elapsedSeconds)")
    $lines.Add("- OS: $($report.os)")
    $lines.Add("- Architecture: $($report.architecture)")
    $lines.Add("- Rust: $($report.cargo)")
    $lines.Add("- Python: $($report.python.version)")
    $lines.Add("- Syncplay oracle: $LegacyRef at $LegacyOraclePath")
    $lines.Add("")
    $lines.Add("| Step | Status | Seconds | Details |")
    $lines.Add("| --- | --- | ---: | --- |")
    foreach ($result in $Results) {
        $lines.Add("| $(Escape-MarkdownCell $result.Step) | $(Escape-MarkdownCell $result.Status) | $($result.Seconds) | $(Escape-MarkdownCell $result.Details) |")
    }
    $lines | Set-Content -LiteralPath $mdPath -Encoding UTF8

    Write-Host ""
    Write-Host "Server release verification reports" -ForegroundColor Cyan
    Write-Host $jsonPath
    Write-Host $mdPath
}

try {
    $oracle = Ensure-LegacyOracle
    $LegacyOraclePath = $oracle.Path
    $LegacyOracleSource = $oracle.Source

    $pythonBin = [Environment]::GetEnvironmentVariable("SYNCPLAY_PYTHON_BIN", "Process")
    if ([string]::IsNullOrWhiteSpace($pythonBin)) {
        $pythonBin = "python"
    }
    Invoke-ServerReleaseCommand "python prerequisites" $pythonBin @("-c", "import twisted, OpenSSL, service_identity")

    if ($Stage -eq "Prepare") { return }
    if ($WorkspaceReceipt) {
        $sourceSha = (& git rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0) { throw "Cannot resolve receipt source" }
        $platform = if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) { "windows-x86_64" } else { "linux-x86_64" }
        Invoke-ServerReleaseCommand "verify prior default workspace receipt" $pythonBin @("scripts/release_qualification.py", "verify-workspace", "--receipt", $WorkspaceReceipt, "--candidate-sha", $sourceSha, "--platform", $platform, "--expected-run-id", $ReceiptRunId)
    }
    Invoke-CargoStep "fmt" @("fmt", "--all", "--", "--check")
    # Package-only resolution and workspace feature unification are distinct
    # obligations even when some test symbols overlap.
    Invoke-CargoStep "sorotte-server tests" @("test", "--locked", "-p", "sorotte-server")
    Invoke-CargoStep "sorotte-compat tests" @("test", "--locked", "-p", "sorotte-compat")
    if (-not $WorkspaceReceipt -and -not $NoWorkspace) {
        Invoke-CargoStep "workspace tests" @("test", "--locked", "--workspace")
    }
    Invoke-CargoStep `
        "strict live legacy compatibility" `
        @("test", "--locked", "-p", "sorotte-compat", "--all-features", "legacy_server_", "--", "--nocapture") `
        @{
            SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY = "1"
            SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY = "1"
        }

    Invoke-CargoStep "clippy" @("clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings")
    Invoke-CargoStep `
        "strict server release matrix" `
        @("test", "--locked", "-p", "sorotte-server", "--test", "server_release_verify", "--", "--test-threads=1", "--nocapture") `
        @{
            SYNCPLAY_REQUIRE_SERVER_RELEASE_VERIFY = "1"
            SOROTTE_SERVER_RELEASE_VERIFY = "1"
            SYNCPLAY_ASSERT_LEGACY_FANOUT_PARITY = "1"
            SYNCPLAY_REQUIRE_LEGACY_TLS_PARITY = "1"
        }
    Invoke-ServerReleaseCommand "verify final legacy reference" $pythonBin @("scripts/release_qualification.py", "verify-legacy", "--legacy-root", $LegacyOraclePath)
    if ($WorkspaceReceipt) {
        Invoke-ServerReleaseCommand "verify final immutable input closure" $pythonBin @("scripts/release_qualification.py", "verify-workspace", "--receipt", $WorkspaceReceipt, "--candidate-sha", $sourceSha, "--platform", $platform, "--expected-run-id", $ReceiptRunId)
    }
}
catch {
    $Failure = $_
}
finally {
    Write-Host ""
    Write-Host "Server release verification matrix" -ForegroundColor Cyan
    $Results | Format-Table Step, Status, Seconds -AutoSize | Out-String | Write-Host
    Write-ServerReleaseReports
}

if ($null -ne $Failure) {
    throw $Failure
}
