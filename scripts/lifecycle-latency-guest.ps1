$ErrorActionPreference = 'Stop'
if ([Environment]::UserName -ine 'WDAGUtilityAccount') { throw 'Run only in Windows Sandbox.' }
$bios = Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\BIOS'
if ($bios.SystemProductName -ne 'Virtual Machine') { throw 'Sandbox VM identity missing.' }
$inputRoot = 'C:\LatencyInput'
$outputRoot = 'C:\LatencyOutput'
$workRoot = 'C:\LatencyWork'
New-Item -ItemType Directory -Path $workRoot | Out-Null
$manifest = Get-Content "$inputRoot\manifest.json" -Raw | ConvertFrom-Json
foreach ($file in $manifest.files.PSObject.Properties) {
    if ((Get-FileHash -LiteralPath (Join-Path $inputRoot $file.Name)).Hash.ToLowerInvariant() -ne $file.Value) {
        throw "Payload hash mismatch: $($file.Name)"
    }
}
Copy-Item -Path "$inputRoot\*" -Destination $workRoot
. "$workRoot\process.ps1"
$env:SOROTTE_TEST_MPV_BIN = "$workRoot\mpv.exe"
$env:SOROTTE_TEST_MEDIA = "$workRoot\generated.mkv"
$env:SOROTTE_TEST_MEDIA_TOOLS = $workRoot
$results = [ordered]@{ source_sha = $manifest.source_sha; profile = $manifest.profile; started_at_utc = [DateTime]::UtcNow.ToString('o'); runs = @(); status = 'running' }
$results | ConvertTo-Json -Depth 6 | Set-Content "$outputRoot\started.json" -Encoding utf8
function Invoke-Probe([string]$Name, [string]$Binary, [string]$Filter) {
    $env:SOROTTE_LATENCY_OUTPUT = Join-Path $workRoot $Name
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $probe = Invoke-CapturedProcess -FilePath (Join-Path $workRoot $Binary) -Arguments @($Filter,'--ignored','--nocapture','--test-threads=1') -WorkingDirectory $workRoot -ProcessTimeoutMs 180000 -StdoutPath "$workRoot\$Name.stdout.log" -StderrPath "$workRoot\$Name.stderr.log"
    $results.runs += [ordered]@{name=$Name; exit_code=$probe.exit_code; elapsed_ms=$watch.ElapsedMilliseconds; timed_out=$probe.timed_out}
    $results | ConvertTo-Json -Depth 6 | Set-Content "$outputRoot\progress.json" -Encoding utf8
    Copy-Item "$workRoot\$Name.*.log" $outputRoot
    $destination = Join-Path $outputRoot $Name
    New-Item -ItemType Directory -Force $destination | Out-Null
    if (Test-Path $env:SOROTTE_LATENCY_OUTPUT) {
        Get-ChildItem $env:SOROTTE_LATENCY_OUTPUT -File | Where-Object { $_.Extension -in @('.json','.log') } | Copy-Item -Destination $destination
    }
}
try {
    Invoke-Probe 'index-baseline' 'index-probe.exe' 'latency_review_index_transaction_scaling'
    $env:SOROTTE_REVIEW_MM = 'off'
    if ($manifest.scenarios -contains 'gui-off') { Invoke-Probe 'gui-off' 'gui-probe.exe' 'latency_review_real_players' }
    $env:SOROTTE_REVIEW_MM = 'on'
    if ($manifest.scenarios -contains 'gui-on-empty') { Invoke-Probe 'gui-on-empty' 'gui-probe.exe' 'latency_review_real_players' }
    $env:SOROTTE_REVIEW_SEED_INDEX = "$workRoot\index-baseline\index-98"
    if ($manifest.scenarios -contains 'gui-on-98mib') { Invoke-Probe 'gui-on-98mib' 'gui-probe.exe' 'latency_review_real_players' }
    $env:SOROTTE_REVIEW_REQUIRED_INDEX = '1'
    $env:SOROTTE_TEST_MEDIA = "$workRoot\generated-required.mkv"
    if ($manifest.scenarios -contains 'gui-required-index-98mib') { Invoke-Probe 'gui-required-index-98mib' 'gui-probe.exe' 'latency_review_real_players' }
    $results.status = if (@($results.runs | Where-Object exit_code -ne 0).Count) { 'failed' } else { 'passed' }
}
catch {
    $results.status = 'failed'
    $results.error = $_.Exception.Message
}
finally {
    $results.finished_at_utc = [DateTime]::UtcNow.ToString('o')
    Get-ChildItem $workRoot -File -Filter '*.log' | Copy-Item -Destination $outputRoot -Force
    $results | ConvertTo-Json -Depth 6 | Set-Content "$outputRoot\completion.json" -Encoding utf8
}
