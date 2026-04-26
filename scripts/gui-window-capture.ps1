param(
    [string[]]$View = @("setup"),
    [string]$SampleUrls = "https://example.com/watch-one`nhttps://example.com/watch-two",
    [string]$BinaryPath,
    [string]$OutputDir = "target/gui-captures",
    [int]$TimeoutMs = 20000,
    [int]$WindowX = 32,
    [int]$WindowY = 32,
    [int]$WindowWidth = 1440,
    [int]$WindowHeight = 900,
    [switch]$NoBuild,
    [switch]$KeepOpen
)

$ErrorActionPreference = "Stop"

$allowedViews = @("setup", "room", "room-change", "plugins", "playlist-urls")
$captureViews = @()
foreach ($viewEntry in $View) {
    foreach ($viewPart in ($viewEntry -split ",")) {
        $normalizedView = $viewPart.Trim().ToLowerInvariant()
        if ($normalizedView.Length -eq 0) {
            continue
        }
        if ($allowedViews -notcontains $normalizedView) {
            throw "Unsupported view '$viewPart'. Expected one of: $($allowedViews -join ', ')"
        }
        $captureViews += $normalizedView
    }
}
if ($captureViews.Count -eq 0) {
    throw "At least one view must be provided. Expected one of: $($allowedViews -join ', ')"
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$outputRoot = if ([System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir
} else {
    Join-Path $repoRoot $OutputDir
}
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

if (-not $BinaryPath) {
    $BinaryPath = Join-Path $repoRoot "target\debug\syncplay-gui.exe"
}
$binaryFullPath = if ([System.IO.Path]::IsPathRooted($BinaryPath)) {
    $BinaryPath
} else {
    Join-Path $repoRoot $BinaryPath
}

if (-not $NoBuild) {
    Push-Location $repoRoot
    try {
        & cargo build -p syncplay-gui --bin syncplay-gui
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $binaryFullPath)) {
    throw "syncplay-gui binary does not exist at $binaryFullPath"
}

$captureTypeDefinition = @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;
using System.Windows.Automation;

public static class SyncplayWindowCapture
{
    public struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);

    private sealed class FindWindowState
    {
        public int ProcessId;
        public IntPtr Window = IntPtr.Zero;
    }

    [DllImport("user32.dll")]
    private static extern bool EnumWindows(EnumWindowsProc enumProc, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    [DllImport("user32.dll")]
    private static extern bool IsWindowVisible(IntPtr hwnd);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int maxCount);

    [DllImport("user32.dll")]
    private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);

    [DllImport("user32.dll")]
    private static extern bool ShowWindow(IntPtr hwnd, int command);

    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern bool SetWindowPos(
        IntPtr hwnd,
        IntPtr insertAfter,
        int x,
        int y,
        int cx,
        int cy,
        uint flags);

    [DllImport("user32.dll")]
    private static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern bool PrintWindow(IntPtr hwnd, IntPtr hdcBlt, uint flags);

    [DllImport("user32.dll")]
    private static extern bool SetCursorPos(int x, int y);

    [DllImport("user32.dll")]
    private static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);

    private static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
    private static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
    private const int SW_RESTORE = 9;
    private const uint SWP_SHOWWINDOW = 0x0040;
    private const uint WM_CLOSE = 0x0010;
    private const uint PW_RENDERFULLCONTENT = 0x00000002;
    private const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    private const uint MOUSEEVENTF_LEFTUP = 0x0004;

    public static IntPtr FindMainWindowForProcess(int processId, int timeoutMs)
    {
        DateTime deadline = DateTime.UtcNow.AddMilliseconds(timeoutMs);
        while (DateTime.UtcNow <= deadline)
        {
            IntPtr found = FindMainWindowForProcessOnce(processId);
            if (found != IntPtr.Zero)
            {
                return found;
            }
            Thread.Sleep(50);
        }
        return IntPtr.Zero;
    }

    private static IntPtr FindMainWindowForProcessOnce(int processId)
    {
        FindWindowState state = new FindWindowState { ProcessId = processId };
        GCHandle handle = GCHandle.Alloc(state);
        try
        {
            EnumWindows(delegate (IntPtr hwnd, IntPtr lParam)
            {
                uint hwndProcessId;
                GetWindowThreadProcessId(hwnd, out hwndProcessId);
                if (hwndProcessId != (uint)state.ProcessId || !IsWindowVisible(hwnd))
                {
                    return true;
                }

                StringBuilder title = new StringBuilder(512);
                GetWindowText(hwnd, title, title.Capacity);
                if (title.ToString().Trim().Length == 0)
                {
                    return true;
                }

                state.Window = hwnd;
                return false;
            }, GCHandle.ToIntPtr(handle));
            return state.Window;
        }
        finally
        {
            handle.Free();
        }
    }

    public static Rect PrepareWindow(IntPtr hwnd, int x, int y, int width, int height)
    {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        if (!SetWindowPos(hwnd, HWND_TOPMOST, x, y, width, height, SWP_SHOWWINDOW))
        {
            throw new InvalidOperationException("failed to position the Syncplay GUI window");
        }
        Thread.Sleep(500);
        SetForegroundWindow(hwnd);
        Thread.Sleep(500);
        return GetBounds(hwnd);
    }

    public static void ClearTopmost(IntPtr hwnd)
    {
        Rect rect = GetBounds(hwnd);
        SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            rect.Left,
            rect.Top,
            rect.Right - rect.Left,
            rect.Bottom - rect.Top,
            SWP_SHOWWINDOW);
    }

    public static Rect GetBounds(IntPtr hwnd)
    {
        Rect rect;
        if (!GetWindowRect(hwnd, out rect))
        {
            throw new InvalidOperationException("failed to read the Syncplay GUI window bounds");
        }
        if (rect.Right <= rect.Left || rect.Bottom <= rect.Top)
        {
            throw new InvalidOperationException("Syncplay GUI window bounds were empty");
        }
        return rect;
    }

    public static void CaptureWindow(IntPtr hwnd, string outputPath)
    {
        Rect rect = GetBounds(hwnd);
        int width = rect.Right - rect.Left;
        int height = rect.Bottom - rect.Top;
        using (Bitmap bitmap = new Bitmap(width, height))
        {
            using (Graphics graphics = Graphics.FromImage(bitmap))
            {
                bool rendered = true;
                try
                {
                    SetForegroundWindow(hwnd);
                    Thread.Sleep(200);
                    graphics.CopyFromScreen(rect.Left, rect.Top, 0, 0, new Size(width, height), CopyPixelOperation.SourceCopy);
                }
                catch
                {
                    rendered = false;
                }

                if (!rendered)
                {
                    IntPtr hdc = graphics.GetHdc();
                    try
                    {
                        PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT);
                    }
                    finally
                    {
                        graphics.ReleaseHdc(hdc);
                    }
                }
            }
            bitmap.Save(outputPath, ImageFormat.Png);
        }
    }

    public static void InvokeNamedControl(IntPtr hwnd, string name)
    {
        AutomationElement root = AutomationElement.FromHandle(hwnd);
        if (root == null)
        {
            throw new InvalidOperationException("could not inspect the Syncplay GUI accessibility tree");
        }

        AutomationElement element = root.FindFirst(
            TreeScope.Descendants,
            new PropertyCondition(AutomationElement.NameProperty, name));
        if (element == null)
        {
            throw new InvalidOperationException("could not find Syncplay GUI control named '" + name + "'");
        }

        object invokePattern;
        if (element.TryGetCurrentPattern(InvokePattern.Pattern, out invokePattern))
        {
            ((InvokePattern)invokePattern).Invoke();
            Thread.Sleep(350);
            return;
        }

        object selectionItemPattern;
        if (element.TryGetCurrentPattern(SelectionItemPattern.Pattern, out selectionItemPattern))
        {
            ((SelectionItemPattern)selectionItemPattern).Select();
            Thread.Sleep(350);
            return;
        }

        System.Windows.Rect bounds = element.Current.BoundingRectangle;
        if (bounds.Width <= 0 || bounds.Height <= 0)
        {
            throw new InvalidOperationException("Syncplay GUI control named '" + name + "' had no clickable bounds");
        }
        int x = (int)(bounds.Left + bounds.Width / 2.0);
        int y = (int)(bounds.Top + bounds.Height / 2.0);
        SetCursorPos(x, y);
        Thread.Sleep(80);
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(30);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(350);
    }

    public static void SetNamedEditValue(IntPtr hwnd, string name, string value)
    {
        AutomationElement root = AutomationElement.FromHandle(hwnd);
        if (root == null)
        {
            throw new InvalidOperationException("could not inspect the Syncplay GUI accessibility tree");
        }

        AutomationElement element = root.FindFirst(
            TreeScope.Descendants,
            new AndCondition(
                new PropertyCondition(AutomationElement.NameProperty, name),
                new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Edit)));
        if (element == null)
        {
            element = root.FindFirst(
            TreeScope.Descendants,
            new PropertyCondition(AutomationElement.NameProperty, name));
        }
        if (element == null)
        {
            throw new InvalidOperationException("could not find Syncplay GUI edit control named '" + name + "'");
        }

        try
        {
            element.SetFocus();
            Thread.Sleep(150);
            SendKeys.SendWait("^a");
            Thread.Sleep(80);
            string[] lines = value.Replace("\r\n", "\n").Replace('\r', '\n').Split('\n');
            for (int index = 0; index < lines.Length; index++)
            {
                SendKeys.SendWait(EscapeSendKeys(lines[index]));
                if (index + 1 < lines.Length)
                {
                    SendKeys.SendWait("{ENTER}");
                }
            }
            Thread.Sleep(350);
            return;
        }
        catch
        {
        }

        object valuePattern;
        if (!element.TryGetCurrentPattern(ValuePattern.Pattern, out valuePattern))
        {
            throw new InvalidOperationException("Syncplay GUI edit control named '" + name + "' does not support direct value setting or keyboard focus");
        }

        ((ValuePattern)valuePattern).SetValue(value);
        Thread.Sleep(350);
    }

    public static void SendKeysToWindow(IntPtr hwnd, string keys)
    {
        SetForegroundWindow(hwnd);
        Thread.Sleep(120);
        SendKeys.SendWait(keys);
        Thread.Sleep(250);
    }

    private static string EscapeSendKeys(string value)
    {
        StringBuilder builder = new StringBuilder();
        foreach (char ch in value)
        {
            if ("+^%~(){}[]".IndexOf(ch) >= 0)
            {
                builder.Append('{').Append(ch).Append('}');
            }
            else
            {
                builder.Append(ch);
            }
        }
        return builder.ToString();
    }

    public static void CloseWindow(IntPtr hwnd)
    {
        PostMessage(hwnd, WM_CLOSE, IntPtr.Zero, IntPtr.Zero);
    }
}
"@
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName WindowsBase
Add-Type -AssemblyName System.Windows.Forms
Add-Type -ReferencedAssemblies "System.Drawing","UIAutomationClient","UIAutomationTypes","WindowsBase","System.Windows.Forms" -TypeDefinition $captureTypeDefinition

function Write-CaptureConfig {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ActiveView
    )

    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $configPath = Join-Path $Root "syncplay.ini"
    @"
[client_settings]
name = smoke-user
room = smoke-room
playerPath = C:\Windows\System32\notepad.exe
sharedPlaylistEnabled = True
folderSearchFirstFileTimeout = 3
folderSearchTimeout = 30
folderSearchDoubleCheckInterval = 2.5
folderSearchWarningThreshold = 7.5
checkforupdatesautomatically = false
"@ | Set-Content -LiteralPath $configPath -Encoding UTF8

    $qsettingsRoot = Join-Path $Root "Syncplay"
    New-Item -ItemType Directory -Force -Path $qsettingsRoot | Out-Null
    @"
[MainWindow]
activeView = $ActiveView
"@ | Set-Content -LiteralPath (Join-Path $qsettingsRoot "MainWindow.ini") -Encoding UTF8

    return $configPath
}

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$captures = @()

foreach ($activeView in $captureViews) {
    $seedView = if ($activeView -eq "playlist-urls" -or $activeView -eq "room-change") { "room" } else { $activeView }
    $profileRoot = Join-Path $repoRoot ("target\gui-capture-runtime\{0}-{1}-{2}" -f $timestamp, $PID, $activeView)
    $configPath = Write-CaptureConfig -Root $profileRoot -ActiveView $seedView
    $process = $null
    $hwnd = [IntPtr]::Zero

    try {
        $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $binaryFullPath
        $startInfo.WorkingDirectory = (Split-Path -Parent $binaryFullPath)
        $startInfo.UseShellExecute = $false
        $startInfo.Environment["SYNCPLAY_CLIENT_CONFIG_PATH"] = $configPath
        $startInfo.Environment["SYNCPLAY_GUI_ENABLE_TEST_PLAYER"] = "true"
        if ($activeView -eq "room" -or $activeView -eq "room-change" -or $activeView -eq "playlist-urls") {
            $startInfo.Environment["SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK"] = "true"
            $startInfo.Environment["SYNCPLAY_CLIENT_USERNAME"] = "smoke-user"
            $startInfo.Environment["SYNCPLAY_CLIENT_ROOM"] = "smoke-room"
        }
        $startInfo.Environment["SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS"] = "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]"
        $startInfo.Environment["SYNCPLAY_GUI_UPDATE_CHECK_RESPONSE"] = '{"version-status":"uptodate","version-message":"Syncplay is up to date."}'
        $startInfo.Environment["SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH"] = Join-Path $profileRoot "media-search"
        $startInfo.Environment["SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS"] = Join-Path $profileRoot "open-target.mkv"
        New-Item -ItemType Directory -Force -Path $startInfo.Environment["SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH"] | Out-Null
        Set-Content -LiteralPath $startInfo.Environment["SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS"] -Value "open-target" -Encoding ASCII

        $process = [System.Diagnostics.Process]::Start($startInfo)
        $hwnd = [SyncplayWindowCapture]::FindMainWindowForProcess($process.Id, $TimeoutMs)
        if ($hwnd -eq [IntPtr]::Zero) {
            throw "timed out waiting for Syncplay GUI window for pid $($process.Id)"
        }

        $bounds = [SyncplayWindowCapture]::PrepareWindow($hwnd, $WindowX, $WindowY, $WindowWidth, $WindowHeight)
        if ($activeView -eq "room") {
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Room")
        } elseif ($activeView -eq "room-change") {
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Room")
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Change Room")
        } elseif ($activeView -eq "plugins") {
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Plugins")
        } elseif ($activeView -eq "setup") {
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Setup")
        } elseif ($activeView -eq "playlist-urls") {
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Room")
            [SyncplayWindowCapture]::InvokeNamedControl($hwnd, "Paste URLs...")
            [SyncplayWindowCapture]::SetNamedEditValue($hwnd, "URLs", $SampleUrls)
        }
        Start-Sleep -Milliseconds 900

        $outputPath = Join-Path $outputRoot ("syncplay-gui-{0}-{1}x{2}-{3}.png" -f $activeView, $WindowWidth, $WindowHeight, $timestamp)
        [SyncplayWindowCapture]::CaptureWindow($hwnd, $outputPath)

        $captures += [pscustomobject]@{
            view = $activeView
            pid = $process.Id
            path = (Resolve-Path -LiteralPath $outputPath).Path
            width = ($bounds.Right - $bounds.Left)
            height = ($bounds.Bottom - $bounds.Top)
            left = $bounds.Left
            top = $bounds.Top
        }
    } finally {
        if ($hwnd -ne [IntPtr]::Zero) {
            try {
                [SyncplayWindowCapture]::ClearTopmost($hwnd)
            } catch {
            }
        }
        if (-not $KeepOpen -and $hwnd -ne [IntPtr]::Zero) {
            [SyncplayWindowCapture]::CloseWindow($hwnd)
        }
        if (-not $KeepOpen -and $process -ne $null -and -not $process.HasExited) {
            if (-not $process.WaitForExit(3000)) {
                $process.Kill()
                $process.WaitForExit()
            }
        }
        if (-not $KeepOpen) {
            Remove-Item -LiteralPath $profileRoot -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

[pscustomobject]@{
    result = "ok"
    binary = (Resolve-Path -LiteralPath $binaryFullPath).Path
    captures = $captures
} | ConvertTo-Json -Depth 4
