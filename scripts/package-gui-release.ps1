param(
    [string]$OutputDir = "target/gui-release",
    [ValidateSet("stable", "dev")]
    [string]$Channel = "stable",
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
Set-Location $RepoRoot

function Resolve-PackagePath {
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
    if (-not $fullPath.StartsWith($repoPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to mutate path outside repo: $fullPath"
    }
}

function Get-SorotteGuiVersion {
    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq "sorotte-gui" } | Select-Object -First 1
    if ($null -eq $package) {
        throw "sorotte-gui package was not found in cargo metadata"
    }
    return [string]$package.version
}

function Get-GitSha {
    if ($env:GITHUB_SHA) {
        return [string]$env:GITHUB_SHA
    }
    $sha = & git rev-parse HEAD 2>$null
    if ($LASTEXITCODE -ne 0) {
        return "unknown"
    }
    return [string]$sha
}

function Assert-WindowsX64 {
    $runningOnWindows = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    if (-not $runningOnWindows -or $arch -ne "X64") {
        throw "GUI release packaging currently supports Windows x64 only; current platform is $([System.Runtime.InteropServices.RuntimeInformation]::OSDescription) $arch"
    }
}

function Copy-ReleaseFile {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "Required release file not found: $Source"
    }
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
}

Assert-WindowsX64

if (-not $SkipBuild) {
    Write-Host "==> Building sorotte-gui release binaries" -ForegroundColor Cyan
    & cargo build --release -p sorotte-gui --bin sorotte-gui
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build sorotte-gui failed with exit code $LASTEXITCODE"
    }
    & cargo build --release -p sorotte-gui --bin sorotte-gui-updater
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build sorotte-gui-updater failed with exit code $LASTEXITCODE"
    }
}

$version = Get-SorotteGuiVersion
$target = "windows-x86_64"
$packageName = "sorotte-gui-$version-$target"
$archiveFileName = "$packageName.zip"
$symbolsPackageName = "$packageName-symbols"
$symbolsArchiveFileName = "$symbolsPackageName.zip"
$outputRoot = Resolve-PackagePath $OutputDir
$stagingRoot = Join-Path $outputRoot "staging"
$artifactsRoot = Join-Path $outputRoot "artifacts"
$packageRoot = Join-Path $stagingRoot $packageName
$symbolsRoot = Join-Path $stagingRoot $symbolsPackageName
$createdAtUtc = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ", [System.Globalization.CultureInfo]::InvariantCulture)
$gitSha = Get-GitSha

Assert-PathInsideRepo $stagingRoot
Assert-PathInsideRepo $artifactsRoot
if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
if (Test-Path -LiteralPath $symbolsRoot) {
    Remove-Item -LiteralPath $symbolsRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
New-Item -ItemType Directory -Force -Path $artifactsRoot | Out-Null

$releaseDir = Join-Path $RepoRoot "target/release"
Copy-ReleaseFile (Join-Path $releaseDir "sorotte-gui.exe") (Join-Path $packageRoot "sorotte-gui.exe")
Copy-ReleaseFile (Join-Path $releaseDir "sorotte-gui-updater.exe") (Join-Path $packageRoot "sorotte-gui-updater.exe")
Copy-ReleaseFile (Join-Path $RepoRoot "README.md") (Join-Path $packageRoot "README.md")
Copy-ReleaseFile (Join-Path $RepoRoot "LICENSE") (Join-Path $packageRoot "LICENSE")
$packageResources = Join-Path $packageRoot "resources"
New-Item -ItemType Directory -Force -Path $packageResources | Out-Null
Copy-ReleaseFile (Join-Path $RepoRoot "resources/sorotte_syncplayintf.lua") (Join-Path $packageResources "sorotte_syncplayintf.lua")

$pdbPaths = @()
foreach ($pdbName in @("sorotte_gui.pdb", "sorotte-gui.pdb", "sorotte_gui_updater.pdb", "sorotte-gui-updater.pdb")) {
    $pdbPath = Join-Path $releaseDir $pdbName
    if (Test-Path -LiteralPath $pdbPath -PathType Leaf) {
        $pdbPaths += $pdbPath
    }
}

$installFiles = @(
    Get-ChildItem -LiteralPath $packageRoot -Recurse -File | Sort-Object FullName | ForEach-Object {
        $relativePath = $_.FullName.Substring($packageRoot.Length) -replace '^[\\/]+', ''
        $relativePath = $relativePath.Replace('\\', '/')
        $fileHash = Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256
        [ordered]@{
            path = $relativePath
            sha256 = $fileHash.Hash.ToLowerInvariant()
        }
    }
)

$installMarker = [ordered]@{
    schema = "sorotte-gui-install-manifest-v2"
    app = "sorotte-gui"
    channel = $Channel
    version = $version
    git_sha = $gitSha
    created_at_utc = $createdAtUtc
    target = $target
    files = $installFiles
}
$installMarker | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $packageRoot "sorotte-install.json") -Encoding UTF8

$archivePath = Join-Path $artifactsRoot $archiveFileName
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}
$symbolsArchivePath = Join-Path $artifactsRoot $symbolsArchiveFileName
$symbolsChecksumPath = "$symbolsArchivePath.sha256"
if (Test-Path -LiteralPath $symbolsArchivePath) {
    Remove-Item -LiteralPath $symbolsArchivePath -Force
}
if (Test-Path -LiteralPath $symbolsChecksumPath) {
    Remove-Item -LiteralPath $symbolsChecksumPath -Force
}

Write-Host "==> Creating $archivePath" -ForegroundColor Cyan
$packageContents = Get-ChildItem -LiteralPath $packageRoot
Compress-Archive -LiteralPath $packageContents.FullName -DestinationPath $archivePath -Force

$hash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
$checksumPath = "$archivePath.sha256"
"$($hash.Hash.ToLowerInvariant())  $archiveFileName" | Set-Content -LiteralPath $checksumPath -Encoding UTF8

$manifestPath = Join-Path $artifactsRoot "sorotte-update-manifest.json"
$manifest = [ordered]@{
    schema = "sorotte-gui-update-manifest-v1"
    app = "sorotte-gui"
    channel = $Channel
    version = $version
    git_sha = $gitSha
    created_at_utc = $createdAtUtc
    target = $target
    package = $archiveFileName
    sha256 = $hash.Hash.ToLowerInvariant()
}
$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding UTF8

if ($pdbPaths.Count -gt 0) {
    New-Item -ItemType Directory -Force -Path $symbolsRoot | Out-Null
    foreach ($pdbPath in $pdbPaths) {
        Copy-Item -LiteralPath $pdbPath -Destination (Join-Path $symbolsRoot (Split-Path -Leaf $pdbPath)) -Force
    }

    Write-Host "==> Creating $symbolsArchivePath" -ForegroundColor Cyan
    $symbolsContents = Get-ChildItem -LiteralPath $symbolsRoot
    Compress-Archive -LiteralPath $symbolsContents.FullName -DestinationPath $symbolsArchivePath -Force

    $symbolsHash = Get-FileHash -LiteralPath $symbolsArchivePath -Algorithm SHA256
    "$($symbolsHash.Hash.ToLowerInvariant())  $symbolsArchiveFileName" | Set-Content -LiteralPath $symbolsChecksumPath -Encoding UTF8
}

Write-Host ""
Write-Host "GUI release package" -ForegroundColor Cyan
Write-Host $archivePath
Write-Host $checksumPath
Write-Host $manifestPath
if ($pdbPaths.Count -gt 0) {
    Write-Host $symbolsArchivePath
    Write-Host $symbolsChecksumPath
}
