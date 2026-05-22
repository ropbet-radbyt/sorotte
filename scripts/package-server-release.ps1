param(
    [string]$OutputDir = "target/server-release",
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

function Get-SorotteServerVersion {
    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq "sorotte-server" } | Select-Object -First 1
    if ($null -eq $package) {
        throw "sorotte-server package was not found in cargo metadata"
    }
    return [string]$package.version
}

function Get-ReleasePlatform {
    $runningOnWindows = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)
    $runningOnLinux = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Linux)
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString()
    if ($arch -ne "X64") {
        throw "Server release packaging currently supports x86_64 only; current architecture is $arch"
    }
    if ($runningOnWindows) {
        return "windows-x86_64"
    }
    if ($runningOnLinux) {
        return "linux-x86_64"
    }
    throw "Server release packaging currently supports Windows and Linux only"
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

if (-not $SkipBuild) {
    Write-Host "==> Building sorotte-server release binary" -ForegroundColor Cyan
    & cargo build --release -p sorotte-server --bin sorotte-server
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}

$version = Get-SorotteServerVersion
$platform = Get-ReleasePlatform
$packageForWindows = $platform.StartsWith("windows")
$binaryName = if ($packageForWindows) { "sorotte-server.exe" } else { "sorotte-server" }
$packageName = "sorotte-server-$version-$platform"
$outputRoot = Resolve-PackagePath $OutputDir
$stagingRoot = Join-Path $outputRoot "staging"
$artifactsRoot = Join-Path $outputRoot "artifacts"
$packageRoot = Join-Path $stagingRoot $packageName

Assert-PathInsideRepo $stagingRoot
Assert-PathInsideRepo $artifactsRoot
if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
New-Item -ItemType Directory -Force -Path $artifactsRoot | Out-Null

$binarySource = Join-Path (Join-Path $RepoRoot "target/release") $binaryName
Copy-ReleaseFile $binarySource (Join-Path $packageRoot $binaryName)
Copy-ReleaseFile (Join-Path $RepoRoot "README.md") (Join-Path $packageRoot "README.md")
Copy-ReleaseFile (Join-Path $RepoRoot "docs/SERVER_RELEASE.md") (Join-Path $packageRoot "SERVER_RELEASE.md")
Copy-ReleaseFile (Join-Path $RepoRoot "LICENSE") (Join-Path $packageRoot "LICENSE")

if ($packageForWindows) {
    $pdbPath = Join-Path $RepoRoot "target/release/sorotte_server.pdb"
    if (Test-Path -LiteralPath $pdbPath -PathType Leaf) {
        Copy-Item -LiteralPath $pdbPath -Destination (Join-Path $packageRoot "sorotte_server.pdb") -Force
    }
}

$archivePath = if ($packageForWindows) {
    Join-Path $artifactsRoot "$packageName.zip"
} else {
    Join-Path $artifactsRoot "$packageName.tar.gz"
}
if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}

Write-Host "==> Creating $archivePath" -ForegroundColor Cyan
if ($packageForWindows) {
    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archivePath -Force
} else {
    & tar -czf $archivePath -C $stagingRoot $packageName
    if ($LASTEXITCODE -ne 0) {
        throw "tar failed with exit code $LASTEXITCODE"
    }
}

$hash = Get-FileHash -LiteralPath $archivePath -Algorithm SHA256
$checksumPath = "$archivePath.sha256"
"$($hash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $archivePath)" | Set-Content -LiteralPath $checksumPath -Encoding UTF8

Write-Host ""
Write-Host "Server release package" -ForegroundColor Cyan
Write-Host $archivePath
Write-Host $checksumPath
