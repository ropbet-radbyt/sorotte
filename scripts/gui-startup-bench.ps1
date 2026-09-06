param(
    [int]$Samples = 20,
    [int]$Warmup = 3,
    [int]$TimeoutMs = 10000,
    [string[]]$Scenario = @("first-run", "configured", "tcp-connect"),
    [string]$BinaryPath,
    [switch]$NoBuild,
    [switch]$Json,
    [switch]$KeepProfileCopy,
    [string]$CompareTo
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repoRoot
try {
    if (-not $NoBuild) {
        & cargo build --locked --release -p sorotte-gui --bin sorotte-gui
        if ($LASTEXITCODE -ne 0) {
            exit $LASTEXITCODE
        }
    }

    if (-not $BinaryPath) {
        $BinaryPath = Join-Path $repoRoot "target\release\sorotte-gui.exe"
    }

    $benchArgs = @(
        "run",
        "--locked",
        "--quiet",
        "-p",
        "sorotte-gui",
        "--bin",
        "sorotte-gui-startup-bench",
        "--",
        "--binary",
        $BinaryPath,
        "--samples",
        [string]$Samples,
        "--warmup",
        [string]$Warmup,
        "--timeout-ms",
        [string]$TimeoutMs
    )

    foreach ($name in $Scenario) {
        if ($name -and $name.Trim().Length -gt 0) {
            $benchArgs += "--scenario"
            $benchArgs += $name
        }
    }

    if ($Json) {
        $benchArgs += "--json"
    }

    if ($KeepProfileCopy) {
        $benchArgs += "--keep-profile-copy"
    }

    if ($CompareTo) {
        $benchArgs += "--compare-to"
        $benchArgs += $CompareTo
    }

    & cargo @benchArgs
    exit $LASTEXITCODE
}
finally {
    Pop-Location
}
