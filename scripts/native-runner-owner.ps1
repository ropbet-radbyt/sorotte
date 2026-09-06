function Get-NativeControllerCommandSha256 {
    param([Parameter(Mandatory=$true)][string]$CommandLine)
    if ([string]::IsNullOrWhiteSpace($CommandLine)) { throw 'Controller command identity is unavailable' }
    $algorithm=[Security.Cryptography.SHA256]::Create()
    try { [BitConverter]::ToString($algorithm.ComputeHash([Text.Encoding]::UTF8.GetBytes($CommandLine))).Replace('-','').ToLowerInvariant() }
    finally { $algorithm.Dispose() }
}

function Assert-NativeControllerOwner {
    param([Parameter(Mandatory=$true)]$Owner,[Parameter(Mandatory=$true)][string]$StartedUtc,[Parameter(Mandatory=$true)][string]$CommandSha256)
    if ($Owner.HasExited) { return }
    if ($Owner.StartTime.ToUniversalTime().ToString('o') -cne $StartedUtc) { throw 'Controller creation identity mismatch; refusing process termination' }
    $identity=@(Get-CimInstance Win32_Process -Filter ('ProcessId='+[string]$Owner.Id) -OperationTimeoutSec 10)
    # The retained handle remains bound to the old process if its PID is reused.
    if ($Owner.HasExited) { return }
    if ($identity.Count -ne 1 -or (Get-NativeControllerCommandSha256 $identity[0].CommandLine) -cne $CommandSha256) { throw 'Controller command identity mismatch; refusing process termination' }
}

function Open-NativeControllerOwner {
    param([Parameter(Mandatory=$true)][int]$ControllerPid,[Parameter(Mandatory=$true)][string]$StartedUtc,[Parameter(Mandatory=$true)][string]$CommandSha256)
    $owner=Get-Process -Id $ControllerPid -ErrorAction SilentlyContinue
    if ($null -eq $owner) { return $null }
    try {
        # Cache the OS process handle before observing identity or deciding to
        # terminate. All subsequent operations use this same process object.
        $null=$owner.Handle
        Assert-NativeControllerOwner -Owner $owner -StartedUtc $StartedUtc -CommandSha256 $CommandSha256
        return $owner
    } catch {
        $failure=$_
        $exited=$false
        try { $exited=$owner.HasExited } catch { }
        $owner.Dispose()
        if ($exited) { return $null }
        throw $failure
    }
}

function Stop-NativeControllerOwner {
    param([Parameter(Mandatory=$true)]$Owner,[Parameter(Mandatory=$true)][string]$StartedUtc,[Parameter(Mandatory=$true)][string]$CommandSha256)
    Assert-NativeControllerOwner -Owner $Owner -StartedUtc $StartedUtc -CommandSha256 $CommandSha256
    if ($Owner.HasExited) { return }
    try { $Owner.Kill() }
    catch { if (-not $Owner.HasExited) { throw } }
    if (-not $Owner.WaitForExit(10000)) { throw 'Owned controller did not stop; cleanup cannot race a live controller' }
}
