param()

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $ScriptDir))
$OriginalLocation = Get-Location
$RepoParent = Split-Path -Parent $RepoRoot
$RepoLeaf = Split-Path -Leaf $RepoRoot
$CollisionRoot = Join-Path $RepoParent "$RepoLeaf-package-path-boundary-$PID"
$HardlinkRoot = Join-Path $RepoRoot "target\package-hardlink-boundary-$PID-$([Guid]::NewGuid().ToString('N'))"
$HardlinkOutsideRoot = Join-Path $RepoParent "$RepoLeaf-package-hardlink-target-$PID-$([Guid]::NewGuid().ToString('N'))"
$ReleaseRoot = Join-Path $RepoRoot "target\release"
$CreatedReleaseRoot = -not (Test-Path -LiteralPath $ReleaseRoot)
$CreatedReleaseFixtures = [System.Collections.Generic.List[string]]::new()

function Get-WorkspacePackageVersion {
    param([Parameter(Mandatory = $true)][string]$PackageName)

    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages |
        Where-Object { $_.name -eq $PackageName } |
        Select-Object -First 1
    if ($null -eq $package) {
        throw "$PackageName package was not found in cargo metadata"
    }
    return [string]$package.version
}

function Assert-PackageScriptRejectsPrefixCollision {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][string]$OutputRoot,
        [Parameter(Mandatory = $true)][string]$PackageName,
        [string]$Channel
    )

    $packageRoot = Join-Path (Join-Path $OutputRoot "staging") $PackageName
    $canaryPath = Join-Path $packageRoot "must-not-be-deleted.txt"
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    Set-Content -LiteralPath $canaryPath -Value "$Name prefix-collision canary" -Encoding UTF8

    $rejectedForBoundary = $false
    $failureMessage = ""
    try {
        if ([string]::IsNullOrEmpty($Channel)) {
            & $ScriptPath -OutputDir $OutputRoot -SkipBuild
        }
        else {
            & $ScriptPath -OutputDir $OutputRoot -SkipBuild -Channel $Channel
        }
    }
    catch {
        $failureMessage = $_.Exception.Message
        $rejectedForBoundary = $failureMessage.Contains("Refusing to mutate path outside repo")
    }
    finally {
        Set-Location $RepoRoot
    }

    if (-not $rejectedForBoundary) {
        throw "$Name packaging did not reject a prefix-collision output path: $failureMessage"
    }
    if (-not (Test-Path -LiteralPath $canaryPath -PathType Leaf)) {
        throw "$Name packaging mutated the prefix-collision path before rejecting it"
    }
}

function Assert-PackageScriptRejectsReparseOutput {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][string]$PackageName,
        [string]$Channel
    )

    $unique = [Guid]::NewGuid().ToString("N")
    $linkRoot = Join-Path $RepoRoot ".package-reparse-output-$unique"
    $outsideRoot = Join-Path $RepoParent "$RepoLeaf-package-reparse-target-$unique"
    $packageRoot = Join-Path (Join-Path $outsideRoot "staging") $PackageName
    $canaryPath = Join-Path $packageRoot "must-not-be-deleted.txt"
    $failureMessage = ""
    $rejectedForBoundary = $false
    $canarySurvived = $false

    try {
        New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
        Set-Content -LiteralPath $canaryPath -Value "$Name reparse-output canary" -Encoding UTF8
        New-Item -ItemType Junction -Path $linkRoot -Target $outsideRoot | Out-Null

        try {
            if ([string]::IsNullOrEmpty($Channel)) {
                & $ScriptPath -OutputDir $linkRoot -SkipBuild
            }
            else {
                & $ScriptPath -OutputDir $linkRoot -SkipBuild -Channel $Channel
            }
        }
        catch {
            $failureMessage = $_.Exception.Message
            $rejectedForBoundary =
                $failureMessage.Contains("Refusing to mutate path outside repo") -or
                $failureMessage.Contains("reparse")
        }
        finally {
            Set-Location $RepoRoot
        }
        $canarySurvived = Test-Path -LiteralPath $canaryPath -PathType Leaf
    }
    finally {
        if (Test-Path -LiteralPath $linkRoot) {
            [System.IO.Directory]::Delete($linkRoot)
        }
        $outsideParent = [System.IO.Path]::GetFullPath((Split-Path -Parent $outsideRoot))
        $outsideLeaf = Split-Path -Leaf $outsideRoot
        if (
            $outsideParent -ne [System.IO.Path]::GetFullPath($RepoParent) -or
            -not $outsideLeaf.StartsWith(
                "$RepoLeaf-package-reparse-target-",
                [System.StringComparison]::Ordinal
            )
        ) {
            throw "Refusing to clean unexpected package reparse target: $outsideRoot"
        }
        if (Test-Path -LiteralPath $outsideRoot) {
            Remove-Item -LiteralPath $outsideRoot -Recurse -Force
        }
    }

    if (-not $canarySurvived) {
        throw "$Name packaging mutated the reparse-point target before rejecting it: $failureMessage"
    }
    if (-not $rejectedForBoundary) {
        throw "$Name packaging did not reject a reparse-point output path: $failureMessage"
    }
}

function Assert-PackageArtifactHardlinksAreReplaced {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ScriptPath,
        [Parameter(Mandatory = $true)][string]$OutputRoot,
        [Parameter(Mandatory = $true)][string[]]$ArtifactRelativePaths,
        [string]$Channel
    )

    $outsideRoot = Join-Path $HardlinkOutsideRoot $Name
    New-Item -ItemType Directory -Force -Path $outsideRoot | Out-Null
    $canaries = @()
    for ($index = 0; $index -lt $ArtifactRelativePaths.Count; $index++) {
        $artifactPath = Join-Path $OutputRoot $ArtifactRelativePaths[$index]
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $artifactPath) | Out-Null
        $canaryPath = Join-Path $outsideRoot "outside-canary-$index.txt"
        $canaryText = "$Name outside hardlink canary $index"
        [System.IO.File]::WriteAllText($canaryPath, $canaryText)
        New-Item -ItemType HardLink -Path $artifactPath -Target $canaryPath | Out-Null
        $canaries += [pscustomobject]@{
            ArtifactPath = $artifactPath
            CanaryPath = $canaryPath
            CanaryText = $canaryText
        }
    }

    try {
        if ([string]::IsNullOrEmpty($Channel)) {
            & $ScriptPath -OutputDir $OutputRoot -SkipBuild
        }
        else {
            & $ScriptPath -OutputDir $OutputRoot -SkipBuild -Channel $Channel
        }
    }
    finally {
        Set-Location $RepoRoot
    }

    foreach ($canary in $canaries) {
        $outsideText = [System.IO.File]::ReadAllText($canary.CanaryPath)
        if ($outsideText -ne $canary.CanaryText) {
            throw "$Name packaging followed an artifact hardlink outside its output directory"
        }
        if (-not (Test-Path -LiteralPath $canary.ArtifactPath -PathType Leaf)) {
            throw "$Name packaging did not produce artifact $($canary.ArtifactPath)"
        }
        if ([System.IO.File]::ReadAllText($canary.ArtifactPath) -eq $canary.CanaryText) {
            throw "$Name packaging left the pre-seeded artifact hardlink in place"
        }
    }
}

try {
    $runningOnWindows = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [System.Runtime.InteropServices.OSPlatform]::Windows
    )
    if (-not $runningOnWindows) {
        throw "Package path boundary regression currently requires Windows because GUI packaging is Windows-only"
    }

    Set-Location $RepoRoot
    $guiVersion = Get-WorkspacePackageVersion "sorotte-gui"
    $serverVersion = Get-WorkspacePackageVersion "sorotte-server"
    New-Item -ItemType Directory -Force -Path $ReleaseRoot | Out-Null
    foreach ($fixtureName in @("sorotte-gui.exe", "sorotte-gui-updater.exe", "sorotte-server.exe")) {
        $fixturePath = Join-Path $ReleaseRoot $fixtureName
        if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) {
            [System.IO.File]::WriteAllText($fixturePath, "$fixtureName package fixture")
            $CreatedReleaseFixtures.Add($fixturePath)
        }
    }
    Assert-PackageScriptRejectsPrefixCollision `
        -Name "GUI" `
        -ScriptPath (Join-Path $ScriptDir "package-gui-release.ps1") `
        -OutputRoot (Join-Path $CollisionRoot "gui") `
        -PackageName "sorotte-gui-$guiVersion-windows-x86_64" `
        -Channel "dev"
    Assert-PackageScriptRejectsPrefixCollision `
        -Name "Server" `
        -ScriptPath (Join-Path $ScriptDir "package-server-release.ps1") `
        -OutputRoot (Join-Path $CollisionRoot "server") `
        -PackageName "sorotte-server-$serverVersion-windows-x86_64"
    Assert-PackageScriptRejectsReparseOutput `
        -Name "Server" `
        -ScriptPath (Join-Path $ScriptDir "package-server-release.ps1") `
        -PackageName "sorotte-server-$serverVersion-windows-x86_64"
    Assert-PackageScriptRejectsReparseOutput `
        -Name "GUI" `
        -ScriptPath (Join-Path $ScriptDir "package-gui-release.ps1") `
        -PackageName "sorotte-gui-$guiVersion-windows-x86_64" `
        -Channel "dev"
    $guiHardlinkOutput = Join-Path $HardlinkRoot "gui"
    Assert-PackageArtifactHardlinksAreReplaced `
        -Name "GUI" `
        -ScriptPath (Join-Path $ScriptDir "package-gui-release.ps1") `
        -OutputRoot $guiHardlinkOutput `
        -ArtifactRelativePaths @(
            "artifacts\sorotte-gui-$guiVersion-windows-x86_64.zip.sha256",
            "artifacts\sorotte-update-manifest.json"
        ) `
        -Channel "dev"
    Assert-PackageArtifactHardlinksAreReplaced `
        -Name "Server" `
        -ScriptPath (Join-Path $ScriptDir "package-server-release.ps1") `
        -OutputRoot (Join-Path $HardlinkRoot "server") `
        -ArtifactRelativePaths @(
            "artifacts\sorotte-server-$serverVersion-windows-x86_64.zip.sha256"
        )

    $guiManifest = Get-Content -Raw -LiteralPath (
        Join-Path $guiHardlinkOutput "artifacts\sorotte-update-manifest.json"
    ) | ConvertFrom-Json
    $sourceCommitTimestamp = (& git show -s --format=%cI $guiManifest.git_sha).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "could not resolve packaged GUI source commit timestamp"
    }
    $expectedCreatedAt = [System.DateTimeOffset]::Parse(
        $sourceCommitTimestamp,
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::RoundtripKind
    ).ToUniversalTime().ToString(
        "yyyy-MM-ddTHH:mm:ssZ",
        [System.Globalization.CultureInfo]::InvariantCulture
    )
    if ($guiManifest.created_at_utc -ne $expectedCreatedAt) {
        throw "dev package freshness must use the source commit timestamp, not rerun time"
    }

    Write-Host "Package path boundary regressions passed."
}
finally {
    Set-Location $OriginalLocation
    $collisionParent = [System.IO.Path]::GetFullPath((Split-Path -Parent $CollisionRoot))
    $collisionLeaf = Split-Path -Leaf $CollisionRoot
    if (
        $collisionParent -ne [System.IO.Path]::GetFullPath($RepoParent) -or
        -not $collisionLeaf.StartsWith(
            "$RepoLeaf-package-path-boundary-",
            [System.StringComparison]::Ordinal
        )
    ) {
        throw "Refusing to clean unexpected package-boundary test path: $CollisionRoot"
    }
    if (Test-Path -LiteralPath $CollisionRoot) {
        Remove-Item -LiteralPath $CollisionRoot -Recurse -Force
    }
    foreach ($fixturePath in $CreatedReleaseFixtures) {
        if (Test-Path -LiteralPath $fixturePath -PathType Leaf) {
            Remove-Item -LiteralPath $fixturePath -Force
        }
    }
    if ($CreatedReleaseRoot -and (Test-Path -LiteralPath $ReleaseRoot)) {
        Remove-Item -LiteralPath $ReleaseRoot -Force
    }
    $resolvedHardlinkRoot = [System.IO.Path]::GetFullPath($HardlinkRoot)
    $expectedHardlinkParent = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "target"))
    if (
        [System.IO.Path]::GetFullPath((Split-Path -Parent $resolvedHardlinkRoot)) -ne
            $expectedHardlinkParent -or
        -not (Split-Path -Leaf $resolvedHardlinkRoot).StartsWith(
            "package-hardlink-boundary-",
            [System.StringComparison]::Ordinal
        )
    ) {
        throw "Refusing to clean unexpected package hardlink output: $resolvedHardlinkRoot"
    }
    if (Test-Path -LiteralPath $resolvedHardlinkRoot) {
        Remove-Item -LiteralPath $resolvedHardlinkRoot -Recurse -Force
    }
    $resolvedHardlinkOutsideRoot = [System.IO.Path]::GetFullPath($HardlinkOutsideRoot)
    if (
        [System.IO.Path]::GetFullPath((Split-Path -Parent $resolvedHardlinkOutsideRoot)) -ne
            [System.IO.Path]::GetFullPath($RepoParent) -or
        -not (Split-Path -Leaf $resolvedHardlinkOutsideRoot).StartsWith(
            "$RepoLeaf-package-hardlink-target-",
            [System.StringComparison]::Ordinal
        )
    ) {
        throw "Refusing to clean unexpected package hardlink target: $resolvedHardlinkOutsideRoot"
    }
    if (Test-Path -LiteralPath $resolvedHardlinkOutsideRoot) {
        Remove-Item -LiteralPath $resolvedHardlinkOutsideRoot -Recurse -Force
    }
}
