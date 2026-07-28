param(
    [Parameter(Mandatory = $true)]
    [string]$ExpectedSha,
    [string]$Remote = "origin",
    [string]$Branch = "main",
    [string]$RepositoryRoot
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = Split-Path -Parent $ScriptDir
}
$RepositoryRoot = [System.IO.Path]::GetFullPath($RepositoryRoot)
$OriginalLocation = Get-Location

try {
    Set-Location $RepositoryRoot
    if ($ExpectedSha -notmatch '^[0-9a-fA-F]{40}$') {
        throw "ExpectedSha must be a full 40-character Git commit SHA"
    }
    if ($Remote -notmatch '^[A-Za-z0-9._/-]+$') {
        throw "Remote contains unsupported characters"
    }
    if ($Branch -notmatch '^[A-Za-z0-9._/-]+$') {
        throw "Branch contains unsupported characters"
    }

    $remoteTrackingRef = "refs/remotes/$Remote/$Branch"
    & git fetch --no-tags $Remote "+refs/heads/${Branch}:$remoteTrackingRef"
    if ($LASTEXITCODE -ne 0) {
        throw "git fetch failed while verifying $Remote/$Branch"
    }
    $currentTip = (& git rev-parse $remoteTrackingRef).Trim()
    if ($LASTEXITCODE -ne 0 -or $currentTip -notmatch '^[0-9a-fA-F]{40}$') {
        throw "could not resolve the current $Remote/$Branch tip"
    }
    if (-not $currentTip.Equals($ExpectedSha, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing stale publication: workflow SHA $ExpectedSha is not current $Remote/$Branch tip $currentTip"
    }

    Write-Host "Verified publication source $ExpectedSha is current $Remote/$Branch."
}
finally {
    Set-Location $OriginalLocation
}
