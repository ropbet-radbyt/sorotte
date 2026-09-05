param(
    [Parameter(Mandatory = $true)][ValidateSet(96, 144, 192)][int]$ExpectedNativeDpi,
    [string]$OutputDir = "target/gui-display-matrix",
    [int]$TimeoutMs = 45000,
    [string]$BinaryPath,
    [switch]$NoBuild
)

# Run on an isolated interactive Windows desktop (docs/DEVELOPMENT.md).
# DPI is a runner setting, independently measured in every scenario window.
$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$targetRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repoRoot "target" }
if (-not $BinaryPath) { $BinaryPath = Join-Path $targetRoot "debug/sorotte-gui.exe" }
if (-not [IO.Path]::IsPathRooted($OutputDir)) { $OutputDir = Join-Path $repoRoot $OutputDir }
$cases = @(
    @{ Id = "narrow-light"; Scenario = "settings.narrow-light"; Theme = "light"; Zoom = 1.0 },
    @{ Id = "wide-dark"; Scenario = "settings.wide-dark"; Theme = "dark"; Zoom = 1.0 },
    @{ Id = "error-zoom"; Scenario = "settings.validation-errors"; Theme = "light"; Zoom = 1.5 },
    @{ Id = "modal-keyboard"; Scenario = "settings.data-danger-zone"; Theme = "dark"; Zoom = 1.0 },
    @{ Id = "content-light"; Scenario = "room.content-stress"; Theme = "light"; Zoom = 1.0 },
    @{ Id = "content-dark-zoom"; Scenario = "room.content-stress"; Theme = "dark"; Zoom = 1.5 }
)
Push-Location $repoRoot
try {
    if (-not $NoBuild) {
        & cargo build --locked -p sorotte-gui --bin sorotte-gui
        if ($LASTEXITCODE -ne 0) { throw "GUI build failed" }
    }
    $conditionRoot = Join-Path $OutputDir "dpi-$ExpectedNativeDpi"
    New-Item -ItemType Directory -Path $conditionRoot -Force | Out-Null
    $rows = @()
    foreach ($case in $cases) {
        $caseRoot = Join-Path $conditionRoot $case.Id
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "gui-visual-suite.ps1") `
            -BinaryPath $BinaryPath -OutputDir $caseRoot -TimeoutMs $TimeoutMs -NoBuild `
            -Scenario $case.Scenario -Theme $case.Theme -UiScale $case.Zoom -ExpectedNativeDpi $ExpectedNativeDpi
        if ($LASTEXITCODE -ne 0) { throw "Display case $($case.Id) failed; partial evidence remains at $caseRoot" }
        $rows += @{ id = $case.Id; scenario = $case.Scenario; theme = $case.Theme; ui_scale = $case.Zoom; directory = $case.Id }
    }
    $report = @{
        schema_version = 1; status = "passed"; input_mode = "strict-physical"
        native_dpi = $ExpectedNativeDpi; binary_sha256 = (Get-FileHash -LiteralPath $BinaryPath -Algorithm SHA256).Hash.ToLowerInvariant()
        required_native_dpi_matrix = @(96, 144, 192); cases = $rows
        scope = "one native DPI profile; each other required profile needs its own measured runner execution"
        screen_reader = "not exercised; UIA and physical keyboard evidence only"
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $conditionRoot "display-matrix.json") -Encoding UTF8
} finally { Pop-Location }
