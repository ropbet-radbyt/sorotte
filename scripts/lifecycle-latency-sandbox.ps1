[CmdletBinding()]
param([Parameter(Mandatory=$true)][string]$RunDirectory,
    [Parameter(Mandatory=$true)][string]$GuiProbePath,
    [Parameter(Mandatory=$true)][string]$IndexProbePath,
    [Parameter(Mandatory=$true)][string]$MpvPath,
    [Parameter(Mandatory=$true)][string]$FfmpegPath,
    [Parameter(Mandatory=$true)][string]$FfprobePath,
    [Parameter(Mandatory=$true)][string]$MediaPath,
    [Parameter(Mandatory=$true)][string]$RequiredIndexMediaPath,
    [string[]]$Scenario = @('gui-off','gui-on-empty','gui-on-98mib','gui-required-index-98mib'))
$ErrorActionPreference = 'Stop'
$Scenario = @($Scenario | ForEach-Object { $_ -split ',' })
if (-not $Scenario.Count -or @($Scenario | Where-Object { $_ -notin @('gui-off','gui-on-empty','gui-on-98mib','gui-required-index-98mib') }).Count) {
    throw 'Select at least one supported GUI scenario.'
}
$repo = Split-Path -Parent $PSScriptRoot
$run = [IO.Path]::GetFullPath($RunDirectory)
$allowed = [IO.Path]::GetFullPath((Join-Path $repo 'target\latency-review')).TrimEnd('\') + '\'
if (-not $run.StartsWith($allowed,[StringComparison]::OrdinalIgnoreCase)) { throw 'Run directory must be under this checkout target\latency-review.' }
if (Test-Path $run) { throw 'Use a fresh run directory.' }
$payload = Join-Path $run 'payload'
$output = Join-Path $run 'output'
$copies = [ordered]@{
    'gui-probe.exe' = $GuiProbePath
    'index-probe.exe' = $IndexProbePath
    'generated.mkv' = $MediaPath
    'generated-required.mkv' = $RequiredIndexMediaPath
    'mpv.exe' = $MpvPath
    'ffmpeg.exe' = $FfmpegPath
    'ffprobe.exe' = $FfprobePath
    'guest.ps1' = Join-Path $PSScriptRoot 'lifecycle-latency-guest.ps1'
    'process.ps1' = Join-Path $PSScriptRoot 'gui-native-smoke-process.ps1'
    'vcruntime140.dll' = Join-Path $env:SystemRoot 'System32\vcruntime140.dll'
    'vcruntime140_1.dll' = Join-Path $env:SystemRoot 'System32\vcruntime140_1.dll'
    'msvcp140.dll' = Join-Path $env:SystemRoot 'System32\msvcp140.dll'
}
$mpvCompiler = Join-Path (Split-Path -Parent $MpvPath) 'd3dcompiler_43.dll'
if (Test-Path -LiteralPath $mpvCompiler -PathType Leaf) { $copies['d3dcompiler_43.dll'] = $mpvCompiler }
foreach ($entry in $copies.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $entry.Value -PathType Leaf)) { throw "Missing $($entry.Key) input: $($entry.Value)" }
}
$before = wsb list --raw | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Sandbox inventory failed.' }
if (@($before.WindowsSandboxEnvironments).Count) { throw 'An existing Sandbox is running.' }
New-Item -ItemType Directory -Path $payload,$output | Out-Null
$files = [ordered]@{}
foreach ($entry in $copies.GetEnumerator()) {
    $destination = Join-Path $payload $entry.Key
    Copy-Item -LiteralPath $entry.Value -Destination $destination
    $files[$entry.Key] = (Get-FileHash -LiteralPath $destination).Hash.ToLowerInvariant()
}
$sourceSha = git -c "safe.directory=$repo" -C $repo rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Source revision lookup failed.' }
$diffPath = Join-Path $run 'experiment.patch'
git -c "safe.directory=$repo" -C $repo diff --no-ext-diff | Set-Content -LiteralPath $diffPath -Encoding utf8
if ($LASTEXITCODE -ne 0) { throw 'Source diff capture failed.' }
$probeSources = [ordered]@{}
foreach ($relative in (git -c "safe.directory=$repo" -C $repo ls-files --others --exclude-standard -- crates)) {
    if ([IO.Path]::GetExtension($relative) -ne '.rs') { continue }
    $source = Join-Path $repo $relative
    $probeSources[$relative] = (Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant()
    $sealed = Join-Path (Join-Path $run 'source') $relative
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $sealed) | Out-Null
    Copy-Item -LiteralPath $source -Destination $sealed
}
@{source_sha=$sourceSha; profile='debug; test-only instrumentation'; scenarios=$Scenario; files=$files; experiment_patch_sha256=(Get-FileHash $diffPath).Hash.ToLowerInvariant(); probe_sources=$probeSources} | ConvertTo-Json -Depth 6 | Set-Content "$payload\manifest.json" -Encoding utf8
$payloadXml = [Security.SecurityElement]::Escape($payload)
$outputXml = [Security.SecurityElement]::Escape($output)
$configuration = @"
<Configuration>
<vGPU>Enable</vGPU><MemoryInMB>8192</MemoryInMB><Networking>Disable</Networking>
<ClipboardRedirection>Disable</ClipboardRedirection><AudioInput>Disable</AudioInput>
<VideoInput>Disable</VideoInput><PrinterRedirection>Disable</PrinterRedirection>
<MappedFolders>
<MappedFolder><HostFolder>$payloadXml</HostFolder><SandboxFolder>C:\LatencyInput</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
<MappedFolder><HostFolder>$outputXml</HostFolder><SandboxFolder>C:\LatencyOutput</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
</MappedFolders>
<LogonCommand><Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\LatencyInput\guest.ps1</Command></LogonCommand>
</Configuration>
"@
$configuration | Set-Content "$run\run.wsb" -Encoding utf8
$id = [guid]::NewGuid().ToString()
$receipt = [ordered]@{sandbox_id=$id; started_at_utc=[DateTime]::UtcNow.ToString('o'); stopped=$false; status='running'}
$receipt | ConvertTo-Json | Set-Content "$run\host-run.json" -Encoding utf8
try {
    wsb start --id $id --config $configuration --raw | Set-Content "$run\start.json" -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw 'Sandbox start failed.' }
    $connector = Start-Process -FilePath (Get-Command wsb.exe).Source -ArgumentList @('connect','--id',$id,'--raw') -WindowStyle Hidden -PassThru
    $deadline = [DateTime]::UtcNow.AddMinutes(15)
    while (-not (Test-Path "$output\completion.json")) {
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Sandbox experiment deadline exceeded.' }
        Start-Sleep -Milliseconds 500
    }
    $completion = Get-Content "$output\completion.json" -Raw | ConvertFrom-Json
    $receipt.status = $completion.status
}
catch { $receipt.status='failed'; $receipt.error=$_.Exception.Message; throw }
finally {
    wsb stop --id $id --raw | Set-Content "$run\stop.json" -Encoding utf8
    $after = wsb list --raw | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Final Sandbox inventory failed.' }
    $after | ConvertTo-Json -Depth 5 | Set-Content "$run\after.json" -Encoding utf8
    $receipt.stopped = $id -notin @($after.WindowsSandboxEnvironments | ForEach-Object Id)
    $receipt.finished_at_utc = [DateTime]::UtcNow.ToString('o')
    $receipt | ConvertTo-Json | Set-Content "$run\host-run.json" -Encoding utf8
}
Write-Output ($receipt | ConvertTo-Json)
if ($receipt.status -ne 'passed' -or -not $receipt.stopped) { throw 'Sandbox experiment or cleanup failed; retain the run receipt.' }
