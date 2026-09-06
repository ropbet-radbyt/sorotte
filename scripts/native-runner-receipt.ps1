# Host-only receipt I/O. Keep the previous complete receipt visible until the
# replacement is durable; Move-Item -Force can remove the destination first.
function Write-NativeRunnerReceipt {
    param([Parameter(Mandatory=$true)][string]$Path,[Parameter(Mandatory=$true)]$Value)
    $pending=$Path+'.pending-'+[Guid]::NewGuid().ToString()
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 12 -Compress))
    try {
        $stream=[IO.FileStream]::new($pending,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try { $stream.Write($bytes,0,$bytes.Length); $stream.Flush($true) }
        finally { $stream.Dispose() }
        $clock=[Diagnostics.Stopwatch]::StartNew()
        while ($true) {
            try {
                if ([IO.File]::Exists($Path)) { [IO.File]::Replace($pending,$Path,[NullString]::Value) }
                else { [IO.File]::Move($pending,$Path) }
                return
            } catch [IO.IOException] {
                # A short-lived observer without FILE_SHARE_DELETE may delay
                # replacement. Never delete the valid receipt to get past it.
                if ($clock.Elapsed.TotalSeconds -ge 3) { throw }
                Start-Sleep -Milliseconds 25
            }
        }
    } finally {
        if ([IO.File]::Exists($pending)) { [IO.File]::Delete($pending) }
    }
}

function Read-NativeRunnerReceipt {
    param([Parameter(Mandatory=$true)][string]$Path)
    $clock=[Diagnostics.Stopwatch]::StartNew()
    while ($true) {
        try {
            # Readers retain the old complete file while an atomic replacement
            # publishes the new one. Default Get-Content sharing prevents this.
            $stream=[IO.FileStream]::new($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
            try {
                $reader=[IO.StreamReader]::new($stream,[Text.Encoding]::UTF8,$true)
                try { $json=$reader.ReadToEnd() }
                finally { $reader.Dispose() }
            } finally { $stream.Dispose() }
            $value=$json | ConvertFrom-Json -ErrorAction Stop
            if ($null -eq $value) { throw 'Native receipt is empty' }
            return $value
        } catch {
            if ($clock.Elapsed.TotalSeconds -ge 1) { throw }
            Start-Sleep -Milliseconds 25
        }
    }
}
