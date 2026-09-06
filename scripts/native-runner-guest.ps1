param(
    [Parameter(Mandatory=$true)][Guid]$InstanceId,
    [Parameter(Mandatory=$true)][string]$ToolsManifestSha256,
    [Parameter(Mandatory=$true)][string]$ScriptSha256,
    [Parameter(Mandatory=$true)][string]$HelperSha256,
    [Parameter(Mandatory=$true)][string]$ExporterSha256
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
if (($identity -split '\\')[-1] -ine 'WDAGUtilityAccount') {
    throw 'This bootstrap runs only in Windows Sandbox.'
}
$computer = Get-CimInstance Win32_ComputerSystem
if ($computer.Manufacturer -ne 'Microsoft Corporation' -or $computer.Model -ne 'Virtual Machine') {
    throw 'This bootstrap runs only in Windows Sandbox.'
}
$inputRoot = 'C:\SorotteCIBootstrap'
$outputRoot = 'C:\SorotteCIOutput'
$workRoot = 'C:\SorotteCI'
$toolsSource = 'C:\SorotteCIToolsSource'
foreach ($entry in @(@($PSCommandPath,$ScriptSha256),@("$inputRoot\tools-manifest.json",$ToolsManifestSha256),@("$inputRoot\gui-native-smoke-process.ps1",$HelperSha256),@("$inputRoot\native_failure_evidence.py",$ExporterSha256))) {
    if ((Get-FileHash -LiteralPath $entry[0] -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry[1]) { throw 'Bootstrap identity mismatch' }
}
if (Test-Path -LiteralPath $workRoot) { throw 'Guest work directory already exists' }
$null = New-Item -ItemType Directory -Path $workRoot
$receipt = [ordered]@{ schema_version=1; kind='sorotte-native-guest-run'; instance=$InstanceId.ToString(); candidate=$null; run=$null; run_attempt=$null; status='preparing'; started=[DateTime]::UtcNow.ToString('o'); finished=$null; runner_exit_code=$null; error=$null }
function Save-Receipt { $receipt | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$outputRoot\completion.json" -Encoding utf8 }
. "$inputRoot\gui-native-smoke-process.ps1"
function Invoke-Tool([string]$Name,[string]$File,[string[]]$Arguments,[int]$TimeoutMs=600000) {
    $result = Invoke-CapturedProcess -FilePath $File -Arguments $Arguments -WorkingDirectory $workRoot -ProcessTimeoutMs $TimeoutMs -StdoutPath "$outputRoot\$Name.stdout.log" -StderrPath "$outputRoot\$Name.stderr.log"
    if ($result.exit_code -ne 0 -or $result.timed_out) { throw "$Name failed; inspect its captured logs" }
}
$runnerRoot = "$workRoot\runner"
$tokenPath = "$outputRoot\registration-token.json"
try {
    $manifest = Get-Content -LiteralPath "$inputRoot\tools-manifest.json" -Raw | ConvertFrom-Json
    if ($manifest.kind -cne 'sorotte-native-runner-inputs' -or $manifest.schema_version -ne 1 -or $manifest.profile.max_jobs -ne 1) { throw 'Unexpected tool manifest' }
    $toolsRoot = "$workRoot\tools"
    & robocopy.exe $toolsSource $toolsRoot /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /NFL /NDL /NJH /NJS /NP > "$outputRoot\tools-copy.log"
    if ($LASTEXITCODE -ge 8) { throw 'Tool copy failed' }
    if (@(Get-ChildItem -LiteralPath $toolsRoot -Recurse -File -Force).Count -ne @($manifest.files.PSObject.Properties).Count) { throw 'Tool file inventory is not closed' }
    foreach ($entry in $manifest.files.PSObject.Properties) {
        $path = [IO.Path]::GetFullPath((Join-Path $toolsRoot $entry.Name))
        if (-not $path.StartsWith($toolsRoot+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Tool path escapes bundle' }
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry.Value) { throw "Tool identity mismatch: $($entry.Name)" }
    }
    $env:CARGO_HOME = "$workRoot\cargo"
    $env:RUSTUP_HOME = "$workRoot\rustup"
    $env:CARGO_BUILD_JOBS = '4'
    $env:PATH = "$toolsRoot\7zip;$toolsRoot\git\bin;$toolsRoot\git\cmd;$toolsRoot\powershell;$toolsRoot\msvc\bin;$toolsRoot\sdk\bin;$toolsRoot\cmake\bin;$toolsRoot\ninja;$env:CARGO_HOME\bin;$env:PATH"
    $env:INCLUDE = "$toolsRoot\msvc\include;$toolsRoot\sdk\include\ucrt;$toolsRoot\sdk\include\shared;$toolsRoot\sdk\include\um;$toolsRoot\sdk\include\winrt"
    $env:LIB = "$toolsRoot\msvc\lib;$toolsRoot\sdk\lib\ucrt;$toolsRoot\sdk\lib\um"
    $env:CC = "$toolsRoot\msvc\bin\cl.exe"
    $env:CXX = $env:CC
    $env:AR = "$toolsRoot\msvc\bin\lib.exe"
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = "$toolsRoot\msvc\bin\link.exe"
    $env:CMAKE_GENERATOR = 'Ninja'
    $env:VSCMD_ARG_TGT_ARCH = 'x64'
    $env:GIT_CONFIG_COUNT = '1'
    $env:GIT_CONFIG_KEY_0 = 'core.longpaths'
    $env:GIT_CONFIG_VALUE_0 = 'true'
    # Trust and tool/cache configuration exist only in this disposable guest.
    $env:GIT_CONFIG_GLOBAL = "$workRoot\empty-gitconfig"
    $env:GIT_CONFIG_NOSYSTEM = '1'
    [IO.File]::WriteAllText($env:GIT_CONFIG_GLOBAL, '')
    $env:RUNNER_TOOL_CACHE = "$workRoot\toolcache"
    $env:AGENT_TOOLSDIRECTORY = $env:RUNNER_TOOL_CACHE
    $pythonVersion = "$env:RUNNER_TOOL_CACHE\Python\$($manifest.profile.python_version)"
    $null = New-Item -ItemType Directory -Path $pythonVersion
    Copy-Item -LiteralPath "$toolsRoot\python312" -Destination "$pythonVersion\x64" -Recurse
    $env:PYTHONHOME = "$pythonVersion\x64"
    $env:PYTHONNOUSERSITE = '1'
    $env:PYTHONDONTWRITEBYTECODE = '1'
    $env:PYTHONIOENCODING = 'utf-8'
    $env:PYTHONPATH = ''
    $env:PATH = "$env:PYTHONHOME;$env:PATH"
    $null = New-Item -ItemType File -Path "$pythonVersion\x64.complete"
    Invoke-Tool 'python-preflight' "$env:PYTHONHOME\python.exe" @('-c','import ssl,sys,unittest; assert ".".join(map(str,sys.version_info[:3])) == sys.argv[1]; print(sys.version); print(ssl.OPENSSL_VERSION)',[string]$manifest.profile.python_version)
    Copy-Item -LiteralPath "$toolsRoot\7zip" -Destination 'C:\Program Files\7-Zip' -Recurse
    Invoke-Tool '7zip-preflight' (Get-Command 7z.exe -ErrorAction Stop).Source @('i')
    $selectedBash = (Get-Command bash.exe -ErrorAction Stop).Source
    if ($selectedBash -ine "$toolsRoot\git\bin\bash.exe") { throw 'Git Bash must resolve through git/bin, never WSL or usr/bin' }
    Invoke-Tool 'bash-preflight' $selectedBash @('--noprofile','--norc','-c','printf sorotte-bash-ready')
    Invoke-Tool 'rust-install' "$toolsRoot\rustup-init.exe" @('-y','--no-modify-path','--profile','minimal','--default-toolchain',[string]$manifest.profile.rust_toolchain) 900000
    [IO.File]::WriteAllText("$workRoot\hello.c", '#include <stdio.h>' + "`n" + 'int main(void) { puts("sorotte-sandbox-msvc-ready"); return 0; }')
    Invoke-Tool 'msvc-build' $env:CC @('/nologo',"$workRoot\hello.c","/Fe:$workRoot\hello-c.exe")
    Invoke-Tool 'msvc-execute' "$workRoot\hello-c.exe" @()
    [IO.File]::WriteAllText("$workRoot\hello.rs", 'fn main() { println!("sorotte-sandbox-rust-ready"); }')
    Invoke-Tool 'rust-build' "$env:CARGO_HOME\bin\rustc.exe" @("$workRoot\hello.rs",'-o',"$workRoot\hello-rust.exe")
    Invoke-Tool 'rust-execute' "$workRoot\hello-rust.exe" @()
    Invoke-Tool 'rust-version' "$env:CARGO_HOME\bin\rustc.exe" @('--version','--verbose')
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SorotteCIDesktop {
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] public static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("wtsapi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool WTSQuerySessionInformationW(IntPtr server, int session, int infoClass, out IntPtr data, out int bytes);
    [DllImport("wtsapi32.dll")] static extern void WTSFreeMemory(IntPtr data);
    public static string SessionName(int session) {
        IntPtr data; int bytes;
        if (!WTSQuerySessionInformationW(IntPtr.Zero,session,6,out data,out bytes)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        try { return Marshal.PtrToStringUni(data) ?? ""; } finally { WTSFreeMemory(data); }
    }
    public static int[] PhysicalSize() {
        IntPtr old=SetThreadDpiAwarenessContext(new IntPtr(-4));
        if (old==IntPtr.Zero) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        try { return new int[]{GetSystemMetrics(0),GetSystemMetrics(1)}; } finally { SetThreadDpiAwarenessContext(old); }
    }
}
'@
    $session = [Diagnostics.Process]::GetCurrentProcess().SessionId
    if ($session -le 0) { throw 'Interactive session required' }
    $station = [SorotteCIDesktop]::SessionName($session)
    if (-not [string]::IsNullOrWhiteSpace($station)) { $env:SESSIONNAME = $station }
    $desktopClock = [Diagnostics.Stopwatch]::StartNew()
    do {
        $explorer = @(Get-Process explorer -ErrorAction SilentlyContinue | Where-Object SessionId -eq $session).Count -gt 0
        $desktop = [SorotteCIDesktop]::OpenInputDesktop(0,$false,0x0100)
        $accessible = $desktop -ne [IntPtr]::Zero
        if ($accessible) { $null = [SorotteCIDesktop]::CloseDesktop($desktop) }
        [uint32]$foregroundPid = 0
        $null = [SorotteCIDesktop]::GetWindowThreadProcessId([SorotteCIDesktop]::GetForegroundWindow(),[ref]$foregroundPid)
        $foreground = if ($foregroundPid) { Get-Process -Id $foregroundPid -ErrorAction SilentlyContinue } else { $null }
        $owned = $null -ne $foreground -and $foreground.SessionId -eq $session
        $size = [SorotteCIDesktop]::PhysicalSize()
        $ready = $explorer -and $accessible -and $owned -and $size[0] -ge 1800 -and $size[1] -ge 1200
        @{user=$identity;instance=$InstanceId.ToString();session=$session;session_name=$station;explorer=$explorer;input_desktop=$accessible;foreground_owned=$owned;physical_width=$size[0];physical_height=$size[1];ready=$ready;os_version=[Environment]::OSVersion.VersionString;tools_manifest_sha256=$ToolsManifestSha256;profile_sha256=$manifest.profile_sha256} | ConvertTo-Json | Set-Content -LiteralPath "$outputRoot\desktop-preflight.json" -Encoding utf8
        if ($ready) { break }
        Start-Sleep -Milliseconds 500
    } while ($desktopClock.Elapsed.TotalSeconds -lt 180)
    if (-not $ready) { throw 'Guest desktop did not become ready' }
    # This capability test is read-only: no keyboard/cursor injection occurs
    # before the trusted one-job assignment is validated.
    Expand-Archive -LiteralPath "$toolsRoot\runner.zip" -DestinationPath $runnerRoot
    $receipt.status = 'ready'
    $receipt | ConvertTo-Json | Set-Content -LiteralPath "$outputRoot\ready.json" -Encoding utf8
    $tokenClock = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $tokenPath)) {
        if ($tokenClock.Elapsed.TotalMinutes -gt 30) { throw 'No registration handoff arrived' }
        Start-Sleep -Seconds 1
    }
    $registration = Get-Content -LiteralPath $tokenPath -Raw | ConvertFrom-Json
    if ($registration.instance -cne $InstanceId.ToString() -or $registration.candidate -notmatch '^[0-9a-f]{40}$' -or $registration.run -le 0 -or $registration.run_attempt -le 0 -or $registration.repository -cne $manifest.profile.repository -or [DateTime]::Parse($registration.expires_at).ToUniversalTime() -le [DateTime]::UtcNow) { throw 'Registration handoff identity mismatch' }
    $receipt.candidate = $registration.candidate
    $receipt.run = $registration.run
    $receipt.run_attempt = $registration.run_attempt
    @{instance=$InstanceId.ToString();candidate=$registration.candidate;run=$registration.run;run_attempt=$registration.run_attempt;repository=$manifest.profile.repository} | ConvertTo-Json | Set-Content -LiteralPath "$workRoot\authorized-jobs.json" -Encoding utf8
    @'
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$policy = Get-Content -LiteralPath 'C:\SorotteCI\authorized-jobs.json' -Raw | ConvertFrom-Json
$actualRun = [long]0
if (-not [long]::TryParse($env:GITHUB_RUN_ID,[ref]$actualRun) -or $actualRun -ne $policy.run -or $env:GITHUB_RUN_ATTEMPT -cne [string]$policy.run_attempt -or $env:GITHUB_SHA -cne $policy.candidate -or $env:GITHUB_REPOSITORY -cne $policy.repository) {
    throw 'Job identity is outside the explicitly authorized release source and run inventory'
}
@{instance=$policy.instance;candidate=$env:GITHUB_SHA;run=$actualRun;run_attempt=$policy.run_attempt;job_key=$env:GITHUB_JOB;repository=$env:GITHUB_REPOSITORY} | ConvertTo-Json | Set-Content -LiteralPath 'C:\SorotteCIOutput\assigned-job.json' -Encoding utf8
Write-Host 'Job source and workflow run match the authorized release assignment.'
'@ | Set-Content -LiteralPath "$workRoot\job-started.ps1" -Encoding utf8
    $env:ACTIONS_RUNNER_HOOK_JOB_STARTED = "$workRoot\job-started.ps1"
    $env:SOROTTE_NATIVE_RUNNER_CONTRACT = 'sorotte-ephemeral-interactive-windows-v1'
    $env:SOROTTE_NATIVE_RUNNER_INSTANCE_ID = $InstanceId.ToString()
    $env:SOROTTE_NATIVE_RUNNER_MAX_JOBS = '1'
    $env:SOROTTE_NATIVE_RUNNER_INPUTS_SHA256 = $ToolsManifestSha256
    Set-Location -LiteralPath $runnerRoot
    & .\config.cmd --url 'https://github.com/ropbet-radbyt/sorotte' --token $registration.token --name "sorotte-sandbox-$InstanceId" --labels 'sorotte-native-interactive,sorotte-ephemeral' --work 'C:\w' --unattended --ephemeral --disableupdate > "$outputRoot\registration.log" 2>&1
    $configurationExit = $LASTEXITCODE
    $registration = $null
    Remove-Item -LiteralPath $tokenPath -Force
    if ($configurationExit -ne 0) { throw 'Runner configuration failed' }
    $jobClock = [Diagnostics.Stopwatch]::StartNew()
    & .\run.cmd > "$outputRoot\listener.log" 2>&1
    $listenerExit = $LASTEXITCODE
    $receipt['runner_wall_seconds'] = $jobClock.Elapsed.TotalSeconds
    if (Test-Path -LiteralPath "$outputRoot\assigned-job.json") { $receipt.run = (Get-Content -LiteralPath "$outputRoot\assigned-job.json" -Raw | ConvertFrom-Json).run }
    $receipt.runner_exit_code = $listenerExit
    if ($listenerExit -ne 0) { throw 'Runner listener failed' }
    $receipt.status = 'job-finished'
} catch {
    $receipt.status = 'failed'
    $receipt.error = $_.Exception.Message
} finally {
    if (Test-Path -LiteralPath $tokenPath) { Remove-Item -LiteralPath $tokenPath -Force }
    # Do not copy runner _diag, configuration, screenshots or arbitrary logs to
    # an upload path. Export safe structured records before the guest is removed.
    try {
        if ($receipt.candidate -and (Test-Path -LiteralPath "$toolsRoot\python312\python.exe")) {
            & "$toolsRoot\python312\python.exe" "$inputRoot\native_failure_evidence.py" export `
                --root 'C:\w' --output "$outputRoot\safe-evidence" --source-sha $receipt.candidate `
                --run-id ([string]$receipt.run) --run-attempt ([string]$receipt.run_attempt) `
                --stage guest-fallback --cleanup pending
            if ($LASTEXITCODE -ne 0) { throw 'Guest diagnostic exporter failed' }
        }
    } catch { $receipt['evidence_export'] = 'unavailable' }
    $receipt.finished = [DateTime]::UtcNow.ToString('o')
    Save-Receipt
}
