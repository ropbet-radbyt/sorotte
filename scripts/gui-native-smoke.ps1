param(
    [switch]$Json,
    [switch]$KeepOpen,
    [string]$BinaryPath,
    [int]$TimeoutMs = 10000,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ExtraArgs = @()
)

$suiteArgs = @()
if ($Json) {
    $suiteArgs += "--json"
}
if ($KeepOpen) {
    $suiteArgs += "--keep-open"
}
if ($BinaryPath) {
    $suiteArgs += "--binary"
    $suiteArgs += $BinaryPath
}
if ($TimeoutMs -gt 0) {
    $suiteArgs += "--timeout-ms"
    $suiteArgs += [string]$TimeoutMs
}
if (-not ($ExtraArgs -contains "--scenario")) {
    $suiteArgs += "--scenario"
    $suiteArgs += "baseline"
    $suiteArgs += "--scenario"
    $suiteArgs += "relaunch"
}
if ($ExtraArgs.Length -gt 0) {
    $suiteArgs += $ExtraArgs
}

$cargoArgs = @(
    "run",
    "--quiet",
    "-p",
    "syncplay-gui",
    "--features",
    "gui-native-smoke,live-python-interop",
    "--bin",
    "syncplay-gui-native-smoke",
    "--"
) + $suiteArgs

& cargo @cargoArgs
exit $LASTEXITCODE
