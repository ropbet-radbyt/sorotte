param(
    [string]$BinaryPath,
    [string]$OutputDir = "target/gui-visual",
    [int]$TimeoutMs = 20000,
    [ValidateRange(1.0, 3.0)][double]$UiScale = 1.0,
    [ValidateRange(0, 384)][int]$ExpectedNativeDpi = 0,
    [ValidateSet("", "light", "dark")][string]$Theme = "",
    [string[]]$Scenario = @(),
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

if (-not $BinaryPath) {
    $BinaryPath = Join-Path $repoRoot "target\debug\sorotte-gui.exe"
}
$binaryFullPath = if ([System.IO.Path]::IsPathRooted($BinaryPath)) {
    $BinaryPath
} else {
    Join-Path $repoRoot $BinaryPath
}
$outputFullPath = if ([System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir
} else {
    Join-Path $repoRoot $OutputDir
}

Push-Location $repoRoot
try {
    if (-not $NoBuild) {
        & cargo build -p sorotte-gui --bin sorotte-gui
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    }
    if (-not (Test-Path -LiteralPath $binaryFullPath -PathType Leaf)) {
        throw "sorotte-gui binary does not exist at $binaryFullPath"
    }

    $suiteArgs = @(
        "run",
        "--quiet",
        "-p",
        "sorotte-gui",
        "--features",
        "gui-native-smoke,live-python-interop",
        "--bin",
        "sorotte-gui-native-smoke",
        "--",
        "--visual-suite",
        "--binary",
        $binaryFullPath,
        "--output-dir",
        $outputFullPath,
        "--timeout-ms",
        [string]$TimeoutMs
    )
    $suiteArgs += @("--ui-scale", $UiScale.ToString([Globalization.CultureInfo]::InvariantCulture))
    if ($ExpectedNativeDpi -ne 0) {
        $suiteArgs += @("--expected-native-dpi", [string]$ExpectedNativeDpi)
    }
    if ($Theme) {
        $suiteArgs += @("--theme", $Theme)
    }
    foreach ($scenarioId in $Scenario) {
        if (-not [string]::IsNullOrWhiteSpace($scenarioId)) {
            $suiteArgs += "--scenario"
            $suiteArgs += $scenarioId.Trim()
        }
    }
    & cargo @suiteArgs
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
