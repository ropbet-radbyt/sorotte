param(
    [switch]$Move,
    [switch]$Force,
    [switch]$DryRun,
    [string]$OldRoot,
    [string]$NewRoot
)

$ErrorActionPreference = "Stop"

function Get-DefaultOldRoot {
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        if (-not $env:APPDATA) {
            throw "APPDATA is not set; pass -OldRoot explicitly."
        }
        return [System.IO.Path]::GetFullPath($env:APPDATA)
    }
    if ($env:XDG_CONFIG_HOME) {
        return [System.IO.Path]::GetFullPath($env:XDG_CONFIG_HOME)
    }
    if (-not $env:HOME) {
        throw "HOME is not set; pass -OldRoot explicitly."
    }
    return [System.IO.Path]::GetFullPath((Join-Path $env:HOME ".config"))
}

function Get-DefaultNewRoot {
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) {
        if (-not $env:APPDATA) {
            throw "APPDATA is not set; pass -NewRoot explicitly."
        }
        return [System.IO.Path]::GetFullPath((Join-Path $env:APPDATA "Sorotte"))
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) {
        if (-not $env:HOME) {
            throw "HOME is not set; pass -NewRoot explicitly."
        }
        return [System.IO.Path]::GetFullPath((Join-Path $env:HOME "Library/Application Support/Sorotte"))
    }
    if ($env:XDG_CONFIG_HOME) {
        return [System.IO.Path]::GetFullPath((Join-Path $env:XDG_CONFIG_HOME "sorotte"))
    }
    if (-not $env:HOME) {
        throw "HOME is not set; pass -NewRoot explicitly."
    }
    return [System.IO.Path]::GetFullPath((Join-Path $env:HOME ".config/sorotte"))
}

function Write-MigrationAction {
    param(
        [Parameter(Mandatory = $true)][string]$Action,
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination
    )

    $verb = if ($Move) { "move" } else { "copy" }
    if ($DryRun) {
        Write-Host "dry-run: $verb $Action from $Source to $Destination"
    } else {
        Write-Host "$verb $Action from $Source to $Destination"
    }
}

function Copy-MigrationFile {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        return $false
    }
    if ((Test-Path -LiteralPath $Destination) -and -not $Force) {
        Write-Host "skip $Label because destination exists: $Destination"
        return $false
    }
    Write-MigrationAction $Label $Source $Destination
    if (-not $DryRun) {
        $parent = Split-Path -Parent $Destination
        if ($parent) {
            New-Item -ItemType Directory -Force -Path $parent | Out-Null
        }
        Copy-Item -LiteralPath $Source -Destination $Destination -Force:$Force
        if ($Move) {
            Remove-Item -LiteralPath $Source -Force
        }
    }
    return $true
}

function Copy-MigrationDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Source,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][string]$Label
    )

    if (-not (Test-Path -LiteralPath $Source -PathType Container)) {
        return $false
    }
    if ((Test-Path -LiteralPath $Destination) -and -not $Force) {
        Write-Host "skip $Label because destination exists: $Destination"
        return $false
    }
    Write-MigrationAction $Label $Source $Destination
    if (-not $DryRun) {
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        foreach ($entry in Get-ChildItem -LiteralPath $Source -Force) {
            Copy-Item -LiteralPath $entry.FullName -Destination $Destination -Recurse -Force:$Force
        }
        if ($Move) {
            Remove-Item -LiteralPath $Source -Recurse -Force
        }
    }
    return $true
}

function Copy-FirstConfig {
    param(
        [Parameter(Mandatory = $true)][string]$OldRootPath,
        [Parameter(Mandatory = $true)][string]$NewRootPath
    )

    $destination = Join-Path $NewRootPath "sorotte.ini"
    $candidates = @(
        (Join-Path $OldRootPath "syncplay.ini"),
        (Join-Path $OldRootPath ".syncplay")
    )
    foreach ($candidate in $candidates) {
        if (Copy-MigrationFile $candidate $destination "stored config") {
            return
        }
    }
}

function Copy-GuiStateStores {
    param(
        [Parameter(Mandatory = $true)][string]$OldRootPath,
        [Parameter(Mandatory = $true)][string]$NewRootPath
    )

    $oldGuiRoot = Join-Path $OldRootPath "Syncplay"
    foreach ($store in @("PlayerList", "MediaBrowseDialog", "MainWindow", "Interface", "MoreSettings")) {
        $destination = Join-Path $NewRootPath "$store.ini"
        $iniSource = Join-Path $oldGuiRoot "$store.ini"
        $confSource = Join-Path $oldGuiRoot "$store.conf"
        if (Copy-MigrationFile $iniSource $destination "GUI state $store") {
            continue
        }
        Copy-MigrationFile $confSource $destination "GUI state $store" | Out-Null
    }
}

function Copy-ManagedStreamHelper {
    param(
        [Parameter(Mandatory = $true)][string]$OldRootPath,
        [Parameter(Mandatory = $true)][string]$NewRootPath
    )

    $source = Join-Path $OldRootPath "Syncplay/tools/stream-helper"
    $destination = Join-Path $NewRootPath "tools/stream-helper"
    Copy-MigrationDirectory $source $destination "managed stream-helper tools" | Out-Null
}

$oldRootPath = if ($OldRoot) { [System.IO.Path]::GetFullPath($OldRoot) } else { Get-DefaultOldRoot }
$newRootPath = if ($NewRoot) { [System.IO.Path]::GetFullPath($NewRoot) } else { Get-DefaultNewRoot }

Write-Host "Old Syncplay root: $oldRootPath"
Write-Host "New Sorotte root: $newRootPath"

Copy-FirstConfig $oldRootPath $newRootPath
Copy-GuiStateStores $oldRootPath $newRootPath
Copy-ManagedStreamHelper $oldRootPath $newRootPath

Write-Host "Migration complete. Sorotte does not read old Syncplay paths at runtime."
