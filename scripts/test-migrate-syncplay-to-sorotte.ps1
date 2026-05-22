$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$MigrationScript = Join-Path $ScriptDir "migrate-syncplay-to-sorotte.ps1"

function New-TestRoot {
    param([Parameter(Mandatory = $true)][string]$Name)
    $suffix = [System.Guid]::NewGuid().ToString("N")
    $root = Join-Path ([System.IO.Path]::GetTempPath()) "sorotte-migration-$Name-$suffix"
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    return $root
}

function Assert-True {
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) {
        throw $Message
    }
}

function Assert-FileText {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Expected,
        [Parameter(Mandatory = $true)][string]$Message
    )
    Assert-True (Test-Path -LiteralPath $Path -PathType Leaf) "$Message (missing $Path)"
    $actual = Get-Content -LiteralPath $Path -Raw
    Assert-True ($actual -eq $Expected) "$Message (expected '$Expected', got '$actual')"
}

$roots = New-Object System.Collections.Generic.List[string]
try {
    $old = New-TestRoot "copy-old"
    $new = New-TestRoot "copy-new"
    $roots.Add($old)
    $roots.Add($new)
    New-Item -ItemType Directory -Force -Path (Join-Path $old "Syncplay/tools/stream-helper/bin") | Out-Null
    Set-Content -LiteralPath (Join-Path $old "syncplay.ini") -Value "host = old.example" -NoNewline
    Set-Content -LiteralPath (Join-Path $old "Syncplay/MainWindow.ini") -Value "[MainWindow]`nactiveView = room`n" -NoNewline
    Set-Content -LiteralPath (Join-Path $old "Syncplay/tools/stream-helper/bin/yt-dlp.exe") -Value "tool" -NoNewline

    & $MigrationScript -OldRoot $old -NewRoot $new | Out-Host
    Assert-FileText (Join-Path $new "sorotte.ini") "host = old.example" "copy should migrate config"
    Assert-True (Test-Path -LiteralPath (Join-Path $new "MainWindow.ini") -PathType Leaf) "copy should migrate GUI state"
    Assert-True (Test-Path -LiteralPath (Join-Path $new "tools/stream-helper/bin/yt-dlp.exe") -PathType Leaf) "copy should migrate stream helper"
    Assert-True (Test-Path -LiteralPath (Join-Path $old "syncplay.ini") -PathType Leaf) "copy mode should leave source config"

    Set-Content -LiteralPath (Join-Path $old "syncplay.ini") -Value "host = replacement.example" -NoNewline
    Set-Content -LiteralPath (Join-Path $new "sorotte.ini") -Value "host = keep.example" -NoNewline
    & $MigrationScript -OldRoot $old -NewRoot $new | Out-Host
    Assert-FileText (Join-Path $new "sorotte.ini") "host = keep.example" "copy without -Force should not overwrite"

    & $MigrationScript -OldRoot $old -NewRoot $new -Force | Out-Host
    Assert-FileText (Join-Path $new "sorotte.ini") "host = replacement.example" "copy with -Force should overwrite"

    $dryOld = New-TestRoot "dry-old"
    $dryNew = Join-Path ([System.IO.Path]::GetTempPath()) "sorotte-migration-dry-new-$([System.Guid]::NewGuid().ToString('N'))"
    $roots.Add($dryOld)
    $roots.Add($dryNew)
    Set-Content -LiteralPath (Join-Path $dryOld "syncplay.ini") -Value "host = dry.example" -NoNewline
    & $MigrationScript -OldRoot $dryOld -NewRoot $dryNew -DryRun | Out-Host
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $dryNew "sorotte.ini"))) "dry-run should not create config"

    $moveOld = New-TestRoot "move-old"
    $moveNew = New-TestRoot "move-new"
    $roots.Add($moveOld)
    $roots.Add($moveNew)
    New-Item -ItemType Directory -Force -Path (Join-Path $moveOld "Syncplay") | Out-Null
    Set-Content -LiteralPath (Join-Path $moveOld ".syncplay") -Value "host = dotfile.example" -NoNewline
    Set-Content -LiteralPath (Join-Path $moveOld "Syncplay/Interface.conf") -Value "[Update]`nlastCheckedQt = now`n" -NoNewline
    & $MigrationScript -OldRoot $moveOld -NewRoot $moveNew -Move | Out-Host
    Assert-FileText (Join-Path $moveNew "sorotte.ini") "host = dotfile.example" "move should migrate dotfile config"
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $moveOld ".syncplay"))) "move should remove source config"
    Assert-True (Test-Path -LiteralPath (Join-Path $moveNew "Interface.ini") -PathType Leaf) "move should convert conf GUI state to ini"

    Write-Host "Migration script tests passed."
} finally {
    foreach ($root in $roots) {
        if ($root -and (Test-Path -LiteralPath $root)) {
            Remove-Item -LiteralPath $root -Recurse -Force
        }
    }
}
