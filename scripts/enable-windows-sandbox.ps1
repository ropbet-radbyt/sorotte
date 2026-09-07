[CmdletBinding()]
param(
    [string]$ReportPath = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\verification\windows-sandbox-feature.json")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$null = New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ReportPath)
$report = [ordered]@{
    schema_version = 1
    feature = "Containers-DisposableClientVM"
    started_at_utc = [DateTime]::UtcNow.ToString("o")
    status = "running"
    previous_state = $null
    current_state = $null
    restart_needed = $false
    error = $null
}
try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run this script in an administrator PowerShell window. It never restarts Windows."
    }
    $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $ReportPath -Encoding utf8
    $feature = Get-WindowsOptionalFeature -Online -FeatureName $report.feature
    $report.previous_state = [string]$feature.State
    if ($feature.State -eq "EnablePending") {
        $report.restart_needed = $true
    }
    elseif ($feature.State -ne "Enabled") {
        $result = Enable-WindowsOptionalFeature -Online -FeatureName $report.feature -All -NoRestart
        $report.restart_needed = [bool]$result.RestartNeeded
    }
    $report.current_state = [string](Get-WindowsOptionalFeature -Online -FeatureName $report.feature).State
    $report.status = if ($report.restart_needed) { "restart-required" } else { "enabled" }
}
catch {
    $report.status = "failed"
    $report.error = $_.Exception.Message
    throw
}
finally {
    $report["finished_at_utc"] = [DateTime]::UtcNow.ToString("o")
    $report | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $ReportPath -Encoding utf8
}
