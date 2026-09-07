param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[0-9a-f]{64}$')]
    [string]$ManifestSha256
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
# Refuse accidental host execution before any desktop operation or file write.
$identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
if (($identity -split '\\')[-1] -ine "WDAGUtilityAccount") {
    throw "This entry point runs only inside Windows Sandbox. Use gui-sandbox-smoke.ps1 on the host."
}
$computer = Get-CimInstance Win32_ComputerSystem
if ($computer.Manufacturer -ne "Microsoft Corporation" -or $computer.Model -ne "Virtual Machine") {
    throw "Windows Sandbox virtual machine identity is missing."
}

$inputRoot = "C:\SorotteSandboxInput"
$outputRoot = "C:\SorotteSandboxOutput"
$workRoot = "C:\SorotteSandboxWork"
$manifestPath = Join-Path $inputRoot "manifest.json"
if ((Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ManifestSha256) {
    throw "Sandbox manifest digest does not match the host launch configuration."
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
if ($manifest.kind -cne "sorotte-windows-sandbox" -or $manifest.schema_version -ne 1) {
    throw "Unsupported sandbox payload."
}
if ($env:COMPUTERNAME -ieq $manifest.host_computer) {
    throw "Refusing to run native input on the originating host."
}
if (@(Get-ChildItem -LiteralPath $outputRoot -Force).Count -ne 0 -or (Test-Path -LiteralPath $workRoot)) {
    throw "Sandbox run was already started; prepare a fresh run instead of reusing its evidence."
}
$result = [ordered]@{
    schema_version = 1
    run_id = $manifest.run_id
    manifest_sha256 = $ManifestSha256
    started_at_utc = [DateTime]::UtcNow.ToString("o")
    status = "running"
    guest_preflight_passed = $false
    binary_sha256_before = @{}
    binary_sha256_after = @{}
    runner = $null
    validator_exit_code = $null
    error = $null
}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputRoot "started.json") -Encoding utf8

function Assert-PayloadInventory {
    param([string]$Root)
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $files = @(Get-ChildItem -LiteralPath $Root -Recurse -Force -File)
    if ($files.Count -ne (@($manifest.files.PSObject.Properties).Count + 1)) {
        throw "Sandbox payload contains missing or unexpected files."
    }
    foreach ($entry in $manifest.files.PSObject.Properties) {
        $path = [IO.Path]::GetFullPath((Join-Path $Root $entry.Name))
        if (-not $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Sandbox payload path escapes its root."
        }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry.Value) {
            throw "Sandbox payload digest mismatch: $($entry.Name)"
        }
    }
}

try {
    Assert-PayloadInventory -Root $inputRoot
    Copy-Item -LiteralPath $inputRoot -Destination $workRoot -Recurse
    Assert-PayloadInventory -Root $workRoot
    . (Join-Path $workRoot "gui-native-smoke-process.ps1")
    $python = Join-Path $workRoot "python\python.exe"
    $env:PYTHONHOME = Join-Path $workRoot "python"
    $env:PYTHONPATH = ""
    $env:PYTHONNOUSERSITE = "1"
    $env:PYTHONDONTWRITEBYTECODE = "1"
    $env:SYNCPLAY_PYTHON_BIN = $python
    $env:SYNCPLAY_LEGACY_ROOT = Join-Path $workRoot "legacy"
    $env:SYNCPLAY_REQUIRE_LIVE_INTEROP = "1"
    $env:PATH = "$env:PYTHONHOME;$env:PATH"
    Expand-Archive -LiteralPath (Join-Path $workRoot "legacy.zip") -DestinationPath $workRoot

    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SorotteSandboxDesktop {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")]
    public static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")]
    public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);

    public static int[] PhysicalScreenSize() {
        // The native harness opts into DPI awareness before querying bounds.
        // PowerShell can be DPI-unaware: query under a temporary per-monitor
        // context so the readiness gate uses the same physical-pixel units.
        IntPtr previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        if (previous == IntPtr.Zero) {
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        }
        try {
            return new int[] { GetSystemMetrics(0), GetSystemMetrics(1) };
        }
        finally {
            SetThreadDpiAwarenessContext(previous);
        }
    }
}
'@
    $sessionId = [Diagnostics.Process]::GetCurrentProcess().SessionId
    if ($sessionId -eq 0) { throw "Sandbox runner cannot run in a service session." }
    # LogonCommand can run before the shell and remote display are ready.
    # Wait for all of the existing desktop requirements, without relaxing them.
    $displayDeadline = [DateTime]::UtcNow.AddSeconds(120)
    do {
        $explorerPresent = @(Get-Process explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $sessionId).Count -gt 0
        $desktop = [SorotteSandboxDesktop]::OpenInputDesktop(0, $false, 0x0100)
        $desktopAccessible = $desktop -ne [IntPtr]::Zero
        if ($desktopAccessible) { $null = [SorotteSandboxDesktop]::CloseDesktop($desktop) }
        $foreground = [SorotteSandboxDesktop]::GetForegroundWindow()
        [uint32]$foregroundPid = 0
        $null = [SorotteSandboxDesktop]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
        $foregroundProcess = if ($foregroundPid) { Get-Process -Id $foregroundPid -ErrorAction SilentlyContinue } else { $null }
        $foregroundOwned = $null -ne $foregroundProcess -and $foregroundProcess.SessionId -eq $sessionId
        # Query live metrics: a cached Screen object can retain the resolution
        # observed before the Sandbox viewer connected or changed size.
        $logicalWidth = [SorotteSandboxDesktop]::GetSystemMetrics(0)
        $logicalHeight = [SorotteSandboxDesktop]::GetSystemMetrics(1)
        $physicalSize = [SorotteSandboxDesktop]::PhysicalScreenSize()
        $width = $physicalSize[0]
        $height = $physicalSize[1]
        $ready = $explorerPresent -and $desktopAccessible -and $foregroundOwned -and $width -ge 1800 -and $height -ge 1200
        [pscustomobject]@{
            user = $identity
            manufacturer = $computer.Manufacturer
            model = $computer.Model
            session_id = $sessionId
            session_name = $env:SESSIONNAME
            explorer_present = $explorerPresent
            input_desktop_accessible = $desktopAccessible
            foreground_pid = $foregroundPid
            foreground_owned = $foregroundOwned
            display_width = $width
            display_height = $height
            display_units = "physical-pixels"
            logical_display_width = $logicalWidth
            logical_display_height = $logicalHeight
            ready = $ready
            os_version = [Environment]::OSVersion.VersionString
            python_version = $manifest.python_version
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputRoot "guest-preflight.json") -Encoding utf8
        if ($ready) { break }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $displayDeadline)
    if (-not $ready) {
        throw "Sandbox desktop was not ready after 120 seconds: explorer=$explorerPresent input_desktop=$desktopAccessible foreground_owned=$foregroundOwned display=${width}x${height}. Keep the viewer open and maximized; inspect guest-preflight.json."
    }
    $result.guest_preflight_passed = $true
    $validator = Join-Path $workRoot "gui_native_smoke_contract.py"
    $requiredScenarios = @(& $python $validator --print-default-scenarios)
    if ($LASTEXITCODE -ne 0 -or ($requiredScenarios -join '|') -cne ($manifest.scenarios -join '|')) {
        throw "Sandbox inventory differs from the strict native validator."
    }
    foreach ($name in @("sorotte-gui.exe", "sorotte-gui-native-smoke.exe")) {
        $result.binary_sha256_before[$name] = (Get-FileHash -LiteralPath (Join-Path $workRoot "bin\$name") -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $gui = Join-Path $workRoot "bin\sorotte-gui.exe"
    $suiteArgs = @("--json", "--binary", $gui, "--input-mode", "strict-physical", "--timeout-ms", [string]$manifest.timeout_ms)
    foreach ($scenario in $requiredScenarios) { $suiteArgs += @("--scenario", $scenario) }
    $result.runner = Invoke-CapturedProcess `
        -FilePath (Join-Path $workRoot "bin\sorotte-gui-native-smoke.exe") `
        -Arguments $suiteArgs -WorkingDirectory $workRoot `
        -StdoutPath (Join-Path $outputRoot "native-report.json") `
        -StderrPath (Join-Path $outputRoot "native-stderr.log") `
        -ProcessTimeoutMs $manifest.wall_clock_timeout_ms `
        -EnvironmentVariables @{ SOROTTE_GUI_NATIVE_SMOKE_ARTIFACT_DIR = $outputRoot }
    foreach ($name in @("sorotte-gui.exe", "sorotte-gui-native-smoke.exe")) {
        $result.binary_sha256_after[$name] = (Get-FileHash -LiteralPath (Join-Path $workRoot "bin\$name") -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($result.binary_sha256_before[$name] -cne $result.binary_sha256_after[$name]) {
            throw "Native executable changed while running: $name"
        }
    }
    $validatorArgs = @(
        $validator, "--input-mode", "strict-physical",
        "--report", (Join-Path $outputRoot "native-report.json"),
        "--stderr", (Join-Path $outputRoot "native-stderr.log"),
        "--summary", (Join-Path $outputRoot "contract-summary.json"),
        "--expected-binary", $gui,
        "--expected-binary-sha256", $result.binary_sha256_before["sorotte-gui.exe"],
        "--producer-exit-code", [string]$result.runner.exit_code
    )
    foreach ($scenario in $requiredScenarios) { $validatorArgs += @("--scenario", $scenario) }
    # The validator intentionally writes its success notice to stderr.
    # PowerShell 5.1 can turn native stderr redirection into a terminating
    # NativeCommandError under ErrorActionPreference=Stop, even on exit 0.
    $validatorResult = Invoke-CapturedProcess -FilePath $python -Arguments $validatorArgs `
        -WorkingDirectory $workRoot -ProcessTimeoutMs 30000 `
        -StdoutPath (Join-Path $outputRoot "validator-stdout.log") `
        -StderrPath (Join-Path $outputRoot "validator.log")
    $result["validator"] = $validatorResult
    $result.validator_exit_code = $validatorResult.exit_code
    if ($result.runner.exit_code -ne 0 -or $result.validator_exit_code -ne 0) {
        throw "Strict native suite or evidence validation failed; inspect native-report.json and native-stderr.log."
    }
    $result.status = "passed"
}
catch {
    $result.status = "failed"
    $result.error = $_.Exception.Message
    $_ | Out-String | Set-Content -LiteralPath (Join-Path $outputRoot "guest-error.log") -Encoding utf8
}
finally {
    $result["finished_at_utc"] = [DateTime]::UtcNow.ToString("o")
    $temporaryResult = Join-Path $outputRoot "completion.tmp"
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $temporaryResult -Encoding utf8
    Move-Item -LiteralPath $temporaryResult -Destination (Join-Path $outputRoot "completion.json")
}
if ($result.status -ne "passed") { exit 1 }
