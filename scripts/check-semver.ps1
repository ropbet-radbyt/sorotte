[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-fA-F]{40}$')]
    [string]$BaselineRev,

    [ValidateSet(
        'sorotte-secret',
        'sorotte-protocol',
        'sorotte-core',
        'sorotte-server',
        'sorotte-media-match',
        'sorotte-client-core',
        'sorotte-client-app',
        'sorotte-player-api',
        'sorotte-player-mpv',
        'sorotte-plex',
        'sorotte-cli',
        'sorotte-gui',
        'sorotte-sim',
        'sorotte-compat'
    )]
    [string[]]$Package = @(
        'sorotte-secret',
        'sorotte-protocol',
        'sorotte-core',
        'sorotte-server',
        'sorotte-media-match',
        'sorotte-client-core',
        'sorotte-client-app',
        'sorotte-player-api',
        'sorotte-player-mpv',
        'sorotte-plex',
        'sorotte-cli',
        'sorotte-gui',
        'sorotte-sim',
        'sorotte-compat'
    ),

    [string]$CargoExecutable = 'cargo'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
)
$targetRoot = [System.IO.Path]::GetFullPath(
    (Join-Path $tempRoot ('sv{0:x}' -f $PID))
)
$comparison = [System.StringComparison]::OrdinalIgnoreCase
$separator = [System.IO.Path]::DirectorySeparatorChar
$tempPrefix = $tempRoot + $separator
$repoPrefix = $repoRoot.TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
) + $separator

if (-not $targetRoot.StartsWith($tempPrefix, $comparison)) {
    throw "Semver target must remain below the system temporary directory: $targetRoot"
}
if ($targetRoot.StartsWith($repoPrefix, $comparison)) {
    throw "Semver target must remain outside the repository: $targetRoot"
}
if ($targetRoot.Length -gt 64) {
    throw "System temporary path is too long for cargo-semver-checks ($($targetRoot.Length) characters): $targetRoot"
}
if (Test-Path -LiteralPath $targetRoot) {
    throw "Refusing to reuse an existing semver target directory: $targetRoot"
}

$hadCargoTarget = Test-Path Env:CARGO_TARGET_DIR
$previousCargoTarget = if ($hadCargoTarget) { $env:CARGO_TARGET_DIR } else { $null }
$locationPushed = $false

try {
    New-Item -ItemType Directory -Path $targetRoot -ErrorAction Stop | Out-Null
    $env:CARGO_TARGET_DIR = $targetRoot
    Push-Location -LiteralPath $repoRoot
    $locationPushed = $true

    foreach ($packageName in $Package) {
        & $CargoExecutable semver-checks --package $packageName --baseline-rev $BaselineRev
        if ($LASTEXITCODE -ne 0) {
            throw "cargo semver-checks failed for $packageName with exit code $LASTEXITCODE"
        }
    }
}
finally {
    if ($locationPushed) {
        Pop-Location
    }
    if ($hadCargoTarget) {
        $env:CARGO_TARGET_DIR = $previousCargoTarget
    }
    else {
        Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    }
    if (Test-Path -LiteralPath $targetRoot) {
        Remove-Item -LiteralPath $targetRoot -Recurse -Force -ErrorAction Stop
    }
}
