param()

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = [System.IO.Path]::GetFullPath((Split-Path -Parent $ScriptDir))
$TestRoot = Join-Path $RepoRoot "target\release-publication-policy-$PID-$([Guid]::NewGuid().ToString('N'))"
$RemoteRoot = Join-Path $TestRoot "remote.git"
$WorkRoot = Join-Path $TestRoot "work"

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)][string]$Repository,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $output = & git -C $Repository @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed: $($output | Out-String)"
    }
    return $output
}

try {
    New-Item -ItemType Directory -Force -Path $TestRoot | Out-Null
    & git init --bare $RemoteRoot | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "test remote initialization failed"
    }
    & git init $WorkRoot | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "test worktree initialization failed"
    }
    Invoke-Git -Repository $WorkRoot -Arguments @("config", "user.name", "Sorotte Test") | Out-Null
    Invoke-Git -Repository $WorkRoot -Arguments @("config", "user.email", "test@sorotte.invalid") | Out-Null
    Invoke-Git -Repository $WorkRoot -Arguments @("branch", "-M", "main") | Out-Null
    Invoke-Git -Repository $WorkRoot -Arguments @("remote", "add", "origin", $RemoteRoot) | Out-Null

    $fixturePath = Join-Path $WorkRoot "fixture.txt"
    [System.IO.File]::WriteAllText($fixturePath, "first")
    Invoke-Git -Repository $WorkRoot -Arguments @("add", "fixture.txt") | Out-Null
    Invoke-Git -Repository $WorkRoot -Arguments @("commit", "-m", "first") | Out-Null
    $firstSha = (Invoke-Git -Repository $WorkRoot -Arguments @("rev-parse", "HEAD")).Trim()
    Invoke-Git -Repository $WorkRoot -Arguments @("push", "-u", "origin", "main") | Out-Null

    [System.IO.File]::WriteAllText($fixturePath, "second")
    Invoke-Git -Repository $WorkRoot -Arguments @("add", "fixture.txt") | Out-Null
    Invoke-Git -Repository $WorkRoot -Arguments @("commit", "-m", "second") | Out-Null
    $secondSha = (Invoke-Git -Repository $WorkRoot -Arguments @("rev-parse", "HEAD")).Trim()
    Invoke-Git -Repository $WorkRoot -Arguments @("push", "origin", "main") | Out-Null

    $staleRejected = $false
    try {
        & (Join-Path $ScriptDir "assert-github-source-tip.ps1") `
            -ExpectedSha $firstSha `
            -RepositoryRoot $WorkRoot `
            -Remote origin `
            -Branch main
    }
    catch {
        $staleRejected = $_.Exception.Message.Contains("refusing stale publication")
    }
    if (-not $staleRejected) {
        throw "an ancestor workflow SHA was not rejected as a stale publication source"
    }
    & (Join-Path $ScriptDir "assert-github-source-tip.ps1") `
        -ExpectedSha $secondSha `
        -RepositoryRoot $WorkRoot `
        -Remote origin `
        -Branch main

    $guiWorkflow = Get-Content -Raw -LiteralPath (
        Join-Path $RepoRoot ".github\workflows\sorotte-gui-release.yml"
    )
    if (-not $guiWorkflow.Contains("assert-github-source-tip.ps1")) {
        throw "GUI dev publication does not verify the current main tip"
    }

    $serverWorkflow = Get-Content -Raw -LiteralPath (
        Join-Path $RepoRoot ".github\workflows\publish-server-container.yml"
    )
    $explicitLatestPolicy = 'type=raw,value=latest,enable=${{ github.event_name == ''workflow_dispatch'' && github.event.inputs.push_latest == ''true'' }}'
    if (-not $serverWorkflow.Contains($explicitLatestPolicy)) {
        throw "server latest publication is not restricted to explicit workflow dispatch"
    }
    if (-not $serverWorkflow.Contains('default: "false"')) {
        throw "manual server latest promotion must default to disabled"
    }

    Write-Host "Release publication policy regressions passed."
}
finally {
    $resolvedTestRoot = [System.IO.Path]::GetFullPath($TestRoot)
    $expectedParent = [System.IO.Path]::GetFullPath((Join-Path $RepoRoot "target"))
    if (
        [System.IO.Path]::GetFullPath((Split-Path -Parent $resolvedTestRoot)) -ne $expectedParent -or
        -not (Split-Path -Leaf $resolvedTestRoot).StartsWith(
            "release-publication-policy-",
            [System.StringComparison]::Ordinal
        )
    ) {
        throw "Refusing to clean unexpected release policy test root: $resolvedTestRoot"
    }
    if (Test-Path -LiteralPath $resolvedTestRoot) {
        Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
    }
}
