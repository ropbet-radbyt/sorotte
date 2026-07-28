param(
    [switch]$Json,
    [switch]$List,
    [string[]]$Scenario = @(),
    [string]$OutputPath,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ExtraArgs = @()
)

$suiteArgs = @()
if ($Json) {
    $suiteArgs += "--json"
}
if ($List) {
    $suiteArgs += "--list"
}
foreach ($name in $Scenario) {
    $suiteArgs += "--scenario"
    $suiteArgs += $name
}
if ($ExtraArgs.Length -gt 0) {
    $suiteArgs += $ExtraArgs
}

$cargoArgs = @(
    "run",
    "--quiet",
    "-p",
    "sorotte-gui",
    "--features",
    "gui-semantic-smoke,live-python-interop",
    "--bin",
    "sorotte-gui-semantic-suite",
    "--"
) + $suiteArgs

$stdoutPath = Join-Path $env:TEMP ("sorotte-gui-semantic-suite-stdout-" + [guid]::NewGuid().ToString("N") + ".log")
$stderrPath = Join-Path $env:TEMP ("sorotte-gui-semantic-suite-stderr-" + [guid]::NewGuid().ToString("N") + ".log")
$isolatedConfigRoot = Join-Path $env:TEMP ("sorotte-gui-semantic-suite-config-" + [guid]::NewGuid().ToString("N"))
$isolatedConfigPath = Join-Path $isolatedConfigRoot "sorotte.ini"
$previousConfigPath = $env:SOROTTE_CLIENT_CONFIG_PATH

try {
    New-Item -Path $isolatedConfigRoot -ItemType Directory | Out-Null
    $env:SOROTTE_CLIENT_CONFIG_PATH = $isolatedConfigPath
    $process = Start-Process -FilePath "cargo" `
        -ArgumentList $cargoArgs `
        -NoNewWindow `
        -Wait `
        -PassThru `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath
}
finally {
    $env:SOROTTE_CLIENT_CONFIG_PATH = $previousConfigPath
}

$output = @()
if (Test-Path -LiteralPath $stdoutPath) {
    $output += Get-Content -LiteralPath $stdoutPath
}
if (Test-Path -LiteralPath $stderrPath) {
    $output += Get-Content -LiteralPath $stderrPath
}
Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $isolatedConfigRoot -Recurse -Force -ErrorAction SilentlyContinue

$exitCode = $process.ExitCode

if ($OutputPath) {
    $directory = Split-Path -Parent $OutputPath
    if ($directory -and -not (Test-Path -LiteralPath $directory)) {
        New-Item -Path $directory -ItemType Directory | Out-Null
    }
    $output | Set-Content -LiteralPath $OutputPath -Encoding UTF8
}

$output | ForEach-Object { Write-Output $_ }
exit $exitCode
