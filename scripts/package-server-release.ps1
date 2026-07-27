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
    if ($fullPath.Equals($repoPath, $comparison)) {
        return
    }

    $currentPath = $repoPath
    $relativePath = $fullPath.Substring($repoPrefix.Length)
    foreach ($component in ($relativePath -split '[\\/]')) {
        if ([string]::IsNullOrEmpty($component)) {
            continue
        }
        $currentPath = Join-Path $currentPath $component
        if (-not (Test-Path -LiteralPath $currentPath)) {
            break
        }
        $item = Get-Item -Force -LiteralPath $currentPath
        if (
            ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne
            0
        ) {
            throw "Refusing to mutate path through reparse point: $currentPath"
        }
    }
}

function Write-Utf8ArtifactFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )

    $fullPath = [System.IO.Path]::GetFullPath($Path)
    Assert-PathInsideRepo $fullPath
    $directory = Split-Path -Parent $fullPath
    if (-not (Test-Path -LiteralPath $directory -PathType Container)) {
        throw "Artifact directory does not exist: $directory"
    }
    $leaf = Split-Path -Leaf $fullPath
    $temporaryPath = Join-Path $directory ".$leaf.$PID.$([Guid]::NewGuid().ToString('N')).tmp"
    $backupPath = "$temporaryPath.backup"
    Assert-PathInsideRepo $temporaryPath
    Assert-PathInsideRepo $backupPath
    $stream = $null
    try {
        $encoding = [System.Text.UTF8Encoding]::new($false)
        $text = if (
            $Content.EndsWith("`n", [System.StringComparison]::Ordinal)
        ) {
            $Content
        }
        else {
            "$Content$([System.Environment]::NewLine)"
        }
        $bytes = $encoding.GetBytes($text)
        $stream = [System.IO.File]::Open(
            $temporaryPath,
            [System.IO.FileMode]::CreateNew,
            [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::None
        )
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
        $stream.Dispose()
        $stream = $null
        if ([System.IO.File]::Exists($fullPath)) {
            [System.IO.File]::Replace($temporaryPath, $fullPath, $backupPath, $true)
            Remove-Item -LiteralPath $backupPath -Force
        }
        else {
            [System.IO.File]::Move($temporaryPath, $fullPath)
        }
    }
    finally {
        if ($null -ne $stream) {
            $stream.Dispose()
        }
        if (Test-Path -LiteralPath $temporaryPath) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
        if (Test-Path -LiteralPath $backupPath) {
            Remove-Item -LiteralPath $backupPath -Force
        }
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
$symbolsPackageName = "$packageName-symbols"
$outputRoot = Resolve-PackagePath $OutputDir
$stagingRoot = Join-Path $outputRoot "staging"
$artifactsRoot = Join-Path $outputRoot "artifacts"
$packageRoot = Join-Path $stagingRoot $packageName
$symbolsRoot = Join-Path $stagingRoot $symbolsPackageName

Assert-PathInsideRepo $stagingRoot
Assert-PathInsideRepo $artifactsRoot
Assert-PathInsideRepo $packageRoot
Assert-PathInsideRepo $symbolsRoot
if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
if (Test-Path -LiteralPath $symbolsRoot) {
    Remove-Item -LiteralPath $symbolsRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
New-Item -ItemType Directory -Force -Path $artifactsRoot | Out-Null

$binarySource = Join-Path (Join-Path $RepoRoot "target/release") $binaryName
Copy-ReleaseFile $binarySource (Join-Path $packageRoot $binaryName)
Copy-ReleaseFile (Join-Path $RepoRoot "README.md") (Join-Path $packageRoot "README.md")
Copy-ReleaseFile (Join-Path $RepoRoot "docs/SERVER_RELEASE.md") (Join-Path $packageRoot "SERVER_RELEASE.md")
Copy-ReleaseFile (Join-Path $RepoRoot "LICENSE") (Join-Path $packageRoot "LICENSE")

$pdbPath = $null
if ($packageForWindows) {
    $candidatePdbPath = Join-Path $RepoRoot "target/release/sorotte_server.pdb"
    if (Test-Path -LiteralPath $candidatePdbPath -PathType Leaf) {
        $pdbPath = $candidatePdbPath
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
$symbolsArchivePath = Join-Path $artifactsRoot "$symbolsPackageName.zip"
$symbolsChecksumPath = "$symbolsArchivePath.sha256"
if (Test-Path -LiteralPath $symbolsArchivePath) {
    Remove-Item -LiteralPath $symbolsArchivePath -Force
}
if (Test-Path -LiteralPath $symbolsChecksumPath) {
    Remove-Item -LiteralPath $symbolsChecksumPath -Force
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
Write-Utf8ArtifactFile `
    -Path $checksumPath `
    -Content "$($hash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $archivePath)"

if ($null -ne $pdbPath) {
    New-Item -ItemType Directory -Force -Path $symbolsRoot | Out-Null
    Copy-Item -LiteralPath $pdbPath -Destination (Join-Path $symbolsRoot "sorotte_server.pdb") -Force

    Write-Host "==> Creating $symbolsArchivePath" -ForegroundColor Cyan
    $symbolsContents = Get-ChildItem -LiteralPath $symbolsRoot
    Compress-Archive -LiteralPath $symbolsContents.FullName -DestinationPath $symbolsArchivePath -Force

    $symbolsHash = Get-FileHash -LiteralPath $symbolsArchivePath -Algorithm SHA256
    Write-Utf8ArtifactFile `
        -Path $symbolsChecksumPath `
        -Content "$($symbolsHash.Hash.ToLowerInvariant())  $(Split-Path -Leaf $symbolsArchivePath)"
}

Write-Host ""
Write-Host "Server release package" -ForegroundColor Cyan
Write-Host $archivePath
Write-Host $checksumPath
if ($null -ne $pdbPath) {
    Write-Host $symbolsArchivePath
    Write-Host $symbolsChecksumPath
}
