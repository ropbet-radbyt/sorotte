param()

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $ScriptDir))
$OriginalLocation = Get-Location
$RepoParent = Split-Path -Parent $RepoRoot
$RepoLeaf = Split-Path -Leaf $RepoRoot
$CollisionRoot = Join-Path $RepoParent "$RepoLeaf-package-path-boundary-$PID"

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
}
