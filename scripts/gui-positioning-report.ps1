param(
    [string[]]$View = @("setup", "room", "room-change", "plugins", "playlist-urls"),
    [string[]]$Size = @("640x520", "900x700", "1280x820", "1440x900", "1700x1100"),
    [string]$SampleUrls = "https://example.com/watch-one`nhttps://example.com/watch-two",
    [string]$BinaryPath,
    [string]$OutputDir = "target/gui-positioning-report",
    [int]$TimeoutMs = 20000,
    [int]$WindowX = 32,
    [int]$WindowY = 32,
    [switch]$Json,
    [switch]$WarningsAsErrors,
    [switch]$NoBuild,
    [switch]$KeepOpen
)

$ErrorActionPreference = "Stop"

$allowedViews = @("setup", "room", "room-change", "plugins", "playlist-urls")
$defaultThemeLabel = "default"
$minimumUsefulControlSize = 2.0

function Resolve-ReportViews {
    param([string[]]$RawViews)

    $resolved = @()
    foreach ($viewEntry in $RawViews) {
        foreach ($viewPart in ($viewEntry -split ",")) {
            $normalizedView = $viewPart.Trim().ToLowerInvariant()
            if ($normalizedView.Length -eq 0) {
                continue
            }
            if ($allowedViews -notcontains $normalizedView) {
                throw "Unsupported view '$viewPart'. Expected one of: $($allowedViews -join ', ')"
            }
            if ($resolved -notcontains $normalizedView) {
                $resolved += $normalizedView
            }
        }
    }
    if ($resolved.Count -eq 0) {
        throw "At least one view must be provided. Expected one of: $($allowedViews -join ', ')"
    }
    return $resolved
}

function Resolve-ReportSizes {
    param([string[]]$RawSizes)

    $resolved = @()
    foreach ($sizeEntry in $RawSizes) {
        foreach ($sizePart in ($sizeEntry -split ",")) {
            $normalizedSize = $sizePart.Trim().ToLowerInvariant()
            if ($normalizedSize.Length -eq 0) {
                continue
            }
            if ($normalizedSize -notmatch '^(\d+)x(\d+)$') {
                throw "Unsupported size '$sizePart'. Expected WIDTHxHEIGHT, for example 1280x820"
            }
            $width = [int]$Matches[1]
            $height = [int]$Matches[2]
            if ($width -le 0 -or $height -le 0) {
                throw "Unsupported size '$sizePart'. Width and height must be positive"
            }
            $key = "{0}x{1}" -f $width, $height
            if (-not ($resolved | Where-Object { $_.key -eq $key })) {
                $resolved += [pscustomobject]@{
                    key = $key
                    width = $width
                    height = $height
                }
            }
        }
    }
    if ($resolved.Count -eq 0) {
        throw "At least one size must be provided. Expected WIDTHxHEIGHT, for example 1280x820"
    }
    return $resolved
}

function ConvertTo-ReportPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Root
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return Join-Path $Root $Path
}

function ConvertTo-RelativeReportPath {
    param(
        [Parameter(Mandatory = $true)][string]$BasePath,
        [Parameter(Mandatory = $true)][string]$TargetPath
    )

    $baseUri = [System.Uri]::new((Resolve-Path -LiteralPath $BasePath).Path.TrimEnd('\') + '\')
    $targetUri = [System.Uri]::new((Resolve-Path -LiteralPath $TargetPath).Path)
    return [System.Uri]::UnescapeDataString($baseUri.MakeRelativeUri($targetUri).ToString()).Replace('/', '\')
}

function ConvertTo-HtmlText {
    param([AllowNull()][string]$Text)

    return [System.Net.WebUtility]::HtmlEncode($Text)
}

function ConvertTo-MarkdownText {
    param([AllowNull()][string]$Text)

    if ($null -eq $Text) {
        return ""
    }
    return $Text.Replace("|", "\|").Replace("`r", " ").Replace("`n", " ")
}

$captureTypeDefinition = @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;
using System.Windows.Forms;

public sealed class SorottePositioningControlSnapshot
{
    public string Name { get; set; }
    public string AutomationId { get; set; }
    public string ControlType { get; set; }
    public string LocalizedControlType { get; set; }
    public bool IsEnabled { get; set; }
    public bool IsOffscreen { get; set; }
    public double Left { get; set; }
    public double Top { get; set; }
    public double Right { get; set; }
    public double Bottom { get; set; }
    public double Width { get; set; }
    public double Height { get; set; }
    public string RuntimeId { get; set; }
    public string ParentRuntimeId { get; set; }
    public int Depth { get; set; }
}

public static class SorottePositioningProbe
{
    public struct WindowRect
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
    private static extern bool GetWindowRect(IntPtr hwnd, out WindowRect rect);

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

    public static WindowRect PrepareWindow(IntPtr hwnd, int x, int y, int width, int height)
    {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        if (!SetWindowPos(hwnd, HWND_TOPMOST, x, y, width, height, SWP_SHOWWINDOW))
        {
            throw new InvalidOperationException("failed to position the Sorotte GUI window");
        }
        Thread.Sleep(500);
        SetForegroundWindow(hwnd);
        Thread.Sleep(500);
        return GetBounds(hwnd);
    }

    public static void ClearTopmost(IntPtr hwnd)
    {
        WindowRect rect = GetBounds(hwnd);
        SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            rect.Left,
            rect.Top,
            rect.Right - rect.Left,
            rect.Bottom - rect.Top,
            SWP_SHOWWINDOW);
    }

    public static WindowRect GetBounds(IntPtr hwnd)
    {
        WindowRect rect;
        if (!GetWindowRect(hwnd, out rect))
        {
            throw new InvalidOperationException("failed to read the Sorotte GUI window bounds");
        }
        if (rect.Right <= rect.Left || rect.Bottom <= rect.Top)
        {
            throw new InvalidOperationException("Sorotte GUI window bounds were empty");
        }
        return rect;
    }

    public static void CaptureWindow(IntPtr hwnd, string outputPath)
    {
        WindowRect rect = GetBounds(hwnd);
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
            throw new InvalidOperationException("could not inspect the Sorotte GUI accessibility tree");
        }

        AutomationElement element = root.FindFirst(
            TreeScope.Descendants,
            new PropertyCondition(AutomationElement.NameProperty, name));
        if (element == null)
        {
            throw new InvalidOperationException("could not find Sorotte GUI control named '" + name + "'");
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
            throw new InvalidOperationException("Sorotte GUI control named '" + name + "' had no clickable bounds");
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
            throw new InvalidOperationException("could not inspect the Sorotte GUI accessibility tree");
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
            throw new InvalidOperationException("could not find Sorotte GUI edit control named '" + name + "'");
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
            throw new InvalidOperationException("Sorotte GUI edit control named '" + name + "' does not support direct value setting or keyboard focus");
        }

        ((ValuePattern)valuePattern).SetValue(value);
        Thread.Sleep(350);
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

    public static List<SorottePositioningControlSnapshot> CollectControls(IntPtr hwnd)
    {
        AutomationElement root = AutomationElement.FromHandle(hwnd);
        if (root == null)
        {
            throw new InvalidOperationException("could not inspect the Sorotte GUI accessibility tree");
        }

        List<SorottePositioningControlSnapshot> controls = new List<SorottePositioningControlSnapshot>();
        AddControlSnapshot(controls, root, "", 0);
        return controls;
    }

    private static void AddControlSnapshot(
        List<SorottePositioningControlSnapshot> controls,
        AutomationElement element,
        string parentRuntimeId,
        int depth)
    {
        string runtimeId = SafeRuntimeId(element);
        System.Windows.Rect bounds = SafeBounds(element);
        controls.Add(new SorottePositioningControlSnapshot
        {
            Name = SafeName(element),
            AutomationId = SafeAutomationId(element),
            ControlType = SafeControlType(element),
            LocalizedControlType = SafeLocalizedControlType(element),
            IsEnabled = SafeIsEnabled(element),
            IsOffscreen = SafeIsOffscreen(element),
            Left = bounds.Left,
            Top = bounds.Top,
            Right = bounds.Right,
            Bottom = bounds.Bottom,
            Width = bounds.Width,
            Height = bounds.Height,
            RuntimeId = runtimeId,
            ParentRuntimeId = parentRuntimeId,
            Depth = depth,
        });

        AutomationElement child = null;
        try
        {
            child = TreeWalker.ControlViewWalker.GetFirstChild(element);
        }
        catch
        {
            child = null;
        }

        while (child != null)
        {
            AddControlSnapshot(controls, child, runtimeId, depth + 1);
            try
            {
                child = TreeWalker.ControlViewWalker.GetNextSibling(child);
            }
            catch
            {
                child = null;
            }
        }
    }

    private static string SafeRuntimeId(AutomationElement element)
    {
        try
        {
            int[] runtimeId = element.GetRuntimeId();
            if (runtimeId == null || runtimeId.Length == 0)
            {
                return "";
            }
            return string.Join(".", Array.ConvertAll(runtimeId, value => value.ToString()));
        }
        catch
        {
            return "";
        }
    }

    private static string SafeName(AutomationElement element)
    {
        try
        {
            return (element.Current.Name ?? "").Trim();
        }
        catch
        {
            return "";
        }
    }

    private static string SafeAutomationId(AutomationElement element)
    {
        try
        {
            return (element.Current.AutomationId ?? "").Trim();
        }
        catch
        {
            return "";
        }
    }

    private static string SafeControlType(AutomationElement element)
    {
        try
        {
            string programmaticName = element.Current.ControlType.ProgrammaticName ?? "";
            if (programmaticName.StartsWith("ControlType."))
            {
                return programmaticName.Substring("ControlType.".Length);
            }
            return programmaticName;
        }
        catch
        {
            return "";
        }
    }

    private static string SafeLocalizedControlType(AutomationElement element)
    {
        try
        {
            return (element.Current.LocalizedControlType ?? "").Trim();
        }
        catch
        {
            return "";
        }
    }

    private static bool SafeIsEnabled(AutomationElement element)
    {
        try
        {
            return element.Current.IsEnabled;
        }
        catch
        {
            return false;
        }
    }

    private static bool SafeIsOffscreen(AutomationElement element)
    {
        try
        {
            return element.Current.IsOffscreen;
        }
        catch
        {
            return false;
        }
    }

    private static System.Windows.Rect SafeBounds(AutomationElement element)
    {
        try
        {
            System.Windows.Rect bounds = element.Current.BoundingRectangle;
            if (bounds.IsEmpty)
            {
                return new System.Windows.Rect(0, 0, 0, 0);
            }
            return bounds;
        }
        catch
        {
            return new System.Windows.Rect(0, 0, 0, 0);
        }
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

function Write-PositioningConfig {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string]$ActiveView
    )

    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    $configPath = Join-Path $Root "sorotte.ini"
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

    $qsettingsRoot = $Root
    New-Item -ItemType Directory -Force -Path $qsettingsRoot | Out-Null
    @"
[MainWindow]
activeView = $ActiveView
"@ | Set-Content -LiteralPath (Join-Path $qsettingsRoot "MainWindow.ini") -Encoding UTF8

    return $configPath
}

function Get-RequiredControlNames {
    param([Parameter(Mandatory = $true)][string]$ActiveView)

    switch ($ActiveView) {
        "setup" { return @("Setup") }
        "room" { return @("Room") }
        "room-change" { return @("Room", "Change Room") }
        "plugins" { return @("Plugins") }
        "playlist-urls" { return @("Room", "URLs") }
        default { return @() }
    }
}

function Get-ControlLabel {
    param([Parameter(Mandatory = $true)]$Control)

    if ($Control.AutomationId) {
        return $Control.AutomationId
    }
    if ($Control.Name) {
        return $Control.Name
    }
    if ($Control.ControlType) {
        return $Control.ControlType
    }
    return "unnamed-control"
}

function New-PositioningWarning {
    param(
        [Parameter(Mandatory = $true)][string]$Code,
        [Parameter(Mandatory = $true)][string]$Message,
        [Parameter(Mandatory = $true)][string]$View,
        [Parameter(Mandatory = $true)][int]$Width,
        [Parameter(Mandatory = $true)][int]$Height,
        [string[]]$Controls = @()
    )

    return [pscustomobject]@{
        code = $Code
        severity = "warning"
        view = $View
        width = $Width
        height = $Height
        message = $Message
        controls = $Controls
    }
}

function Test-OverlapCandidate {
    param([Parameter(Mandatory = $true)]$Control)

    $kind = [string]$Control.ControlType
    return @(
        "Button",
        "Calendar",
        "CheckBox",
        "ComboBox",
        "Edit",
        "Hyperlink",
        "ListItem",
        "MenuItem",
        "RadioButton",
        "Slider",
        "Spinner",
        "SplitButton",
        "TabItem",
        "TreeItem"
    ) -contains $kind
}

function Get-IntersectionArea {
    param(
        [Parameter(Mandatory = $true)]$A,
        [Parameter(Mandatory = $true)]$B
    )

    $left = [Math]::Max([double]$A.Left, [double]$B.Left)
    $top = [Math]::Max([double]$A.Top, [double]$B.Top)
    $right = [Math]::Min([double]$A.Right, [double]$B.Right)
    $bottom = [Math]::Min([double]$A.Bottom, [double]$B.Bottom)
    if ($right -le $left -or $bottom -le $top) {
        return 0.0
    }
    return (($right - $left) * ($bottom - $top))
}

function Test-ControlIntersectsWindow {
    param(
        [Parameter(Mandatory = $true)]$Control,
        [Parameter(Mandatory = $true)]$WindowBounds
    )

    return [double]$Control.Right -gt [double]$WindowBounds.left `
        -and [double]$Control.Left -lt [double]$WindowBounds.right `
        -and [double]$Control.Bottom -gt [double]$WindowBounds.top `
        -and [double]$Control.Top -lt [double]$WindowBounds.bottom
}

function Get-PositioningWarnings {
    param(
        [Parameter(Mandatory = $true)][string]$ActiveView,
        [Parameter(Mandatory = $true)][int]$RequestedWidth,
        [Parameter(Mandatory = $true)][int]$RequestedHeight,
        [Parameter(Mandatory = $true)]$WindowBounds,
        [Parameter(Mandatory = $true)]$Controls
    )

    $warnings = @()
    $windowTolerance = 2.0

    $requiredControlNames = @(Get-RequiredControlNames -ActiveView $ActiveView)

    foreach ($control in $Controls) {
        $label = Get-ControlLabel -Control $control
        $hasStableLabel = -not [string]::IsNullOrWhiteSpace($control.Name) -or -not [string]::IsNullOrWhiteSpace($control.AutomationId)
        if (-not $hasStableLabel -or $control.IsOffscreen) {
            continue
        }
        $isRequiredControl = $requiredControlNames -contains $control.Name -or $requiredControlNames -contains $control.AutomationId
        $isRequiredWarningCandidate = $isRequiredControl -and [string]$control.ControlType -ne "Text"
        $isActionableControl = Test-OverlapCandidate -Control $control
        if (-not $isRequiredWarningCandidate -and -not $isActionableControl) {
            continue
        }

        if ([double]$control.Width -lt $minimumUsefulControlSize -or [double]$control.Height -lt $minimumUsefulControlSize) {
            $warnings += New-PositioningWarning `
                -Code "empty-bounds" `
                -Message "Visible control '$label' has near-empty bounds." `
                -View $ActiveView `
                -Width $RequestedWidth `
                -Height $RequestedHeight `
                -Controls @($label)
            continue
        }

        if (-not (Test-ControlIntersectsWindow -Control $control -WindowBounds $WindowBounds)) {
            continue
        }

        if (
            [double]$control.Left -lt ([double]$WindowBounds.left - $windowTolerance) -or
            [double]$control.Top -lt ([double]$WindowBounds.top - $windowTolerance) -or
            [double]$control.Right -gt ([double]$WindowBounds.right + $windowTolerance) -or
            [double]$control.Bottom -gt ([double]$WindowBounds.bottom + $windowTolerance)
        ) {
            $warnings += New-PositioningWarning `
                -Code "outside-window" `
                -Message "Visible control '$label' extends outside the captured window bounds." `
                -View $ActiveView `
                -Width $RequestedWidth `
                -Height $RequestedHeight `
                -Controls @($label)
        }
    }

    foreach ($requiredName in $requiredControlNames) {
        $matches = @($Controls | Where-Object { $_.Name -eq $requiredName -or $_.AutomationId -eq $requiredName })
        if ($matches.Count -eq 0) {
            $warnings += New-PositioningWarning `
                -Code "required-missing" `
                -Message "Required control '$requiredName' was not found in the UI Automation tree." `
                -View $ActiveView `
                -Width $RequestedWidth `
                -Height $RequestedHeight `
                -Controls @($requiredName)
            continue
        }
        $visibleMatches = @($matches | Where-Object {
            -not $_.IsOffscreen -and [double]$_.Width -ge $minimumUsefulControlSize -and [double]$_.Height -ge $minimumUsefulControlSize
        })
        if ($visibleMatches.Count -eq 0) {
            $warnings += New-PositioningWarning `
                -Code "required-offscreen" `
                -Message "Required control '$requiredName' exists but is offscreen or has unusable bounds." `
                -View $ActiveView `
                -Width $RequestedWidth `
                -Height $RequestedHeight `
                -Controls @($requiredName)
        }
    }

    $overlapCandidates = @($Controls | Where-Object {
        -not $_.IsOffscreen `
            -and [double]$_.Width -ge 4.0 `
            -and [double]$_.Height -ge 4.0 `
            -and -not [string]::IsNullOrWhiteSpace($_.ParentRuntimeId) `
            -and (Test-ControlIntersectsWindow -Control $_ -WindowBounds $WindowBounds) `
            -and (Test-OverlapCandidate -Control $_)
    })
    $parentGroups = $overlapCandidates | Group-Object -Property ParentRuntimeId
    foreach ($group in $parentGroups) {
        $siblings = @($group.Group)
        if ($siblings.Count -lt 2) {
            continue
        }
        for ($leftIndex = 0; $leftIndex -lt $siblings.Count; $leftIndex++) {
            for ($rightIndex = $leftIndex + 1; $rightIndex -lt $siblings.Count; $rightIndex++) {
                $leftControl = $siblings[$leftIndex]
                $rightControl = $siblings[$rightIndex]
                $intersectionArea = Get-IntersectionArea -A $leftControl -B $rightControl
                if ($intersectionArea -le 16.0) {
                    continue
                }
                $leftArea = [double]$leftControl.Width * [double]$leftControl.Height
                $rightArea = [double]$rightControl.Width * [double]$rightControl.Height
                $smallerArea = [Math]::Min($leftArea, $rightArea)
                if ($smallerArea -le 0.0) {
                    continue
                }
                if (($intersectionArea / $smallerArea) -lt 0.18) {
                    continue
                }

                $leftLabel = Get-ControlLabel -Control $leftControl
                $rightLabel = Get-ControlLabel -Control $rightControl
                $warnings += New-PositioningWarning `
                    -Code "sibling-overlap" `
                    -Message "Sibling controls '$leftLabel' and '$rightLabel' overlap within the same UI Automation parent." `
                    -View $ActiveView `
                    -Width $RequestedWidth `
                    -Height $RequestedHeight `
                    -Controls @($leftLabel, $rightLabel)
            }
        }
    }

    return $warnings
}

function Invoke-PositioningViewSetup {
    param(
        [Parameter(Mandatory = $true)][IntPtr]$Window,
        [Parameter(Mandatory = $true)][string]$ActiveView,
        [Parameter(Mandatory = $true)][string]$SampleUrls
    )

    if ($ActiveView -eq "room") {
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Room")
    } elseif ($ActiveView -eq "room-change") {
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Room")
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Change Room")
    } elseif ($ActiveView -eq "plugins") {
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Plugins")
    } elseif ($ActiveView -eq "setup") {
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Setup")
    } elseif ($ActiveView -eq "playlist-urls") {
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Room")
        [SorottePositioningProbe]::InvokeNamedControl($Window, "Paste URLs...")
        [SorottePositioningProbe]::SetNamedEditValue($Window, "URLs", $SampleUrls)
    }
}

function Write-ReportIndexes {
    param(
        [Parameter(Mandatory = $true)][string]$OutputRoot,
        [Parameter(Mandatory = $true)]$Report
    )

    $markdownPath = Join-Path $OutputRoot "index.md"
    $htmlPath = Join-Path $OutputRoot "index.html"
    $markdownLines = @()
    $markdownLines += "# Sorotte GUI Positioning Report"
    $markdownLines += ""
    $markdownLines += "- Result: $($Report.result)"
    $markdownLines += "- Binary: ``$($Report.binary)``"
    $markdownLines += "- Captures: $($Report.captures.Count)"
    $markdownLines += "- Warnings: $($Report.warnings.Count)"
    $markdownLines += "- Generated: $($Report.generatedAt)"
    $markdownLines += ""
    $markdownLines += "## Warnings"
    $markdownLines += ""
    if ($Report.warnings.Count -eq 0) {
        $markdownLines += "No positioning warnings were reported."
    } else {
        $markdownLines += "| View | Size | Code | Message | Controls |"
        $markdownLines += "| --- | --- | --- | --- | --- |"
        foreach ($warning in $Report.warnings) {
            $controlText = ($warning.controls -join ", ")
            $markdownLines += "| $(ConvertTo-MarkdownText $warning.view) | $($warning.width)x$($warning.height) | $(ConvertTo-MarkdownText $warning.code) | $(ConvertTo-MarkdownText $warning.message) | $(ConvertTo-MarkdownText $controlText) |"
        }
    }
    $markdownLines += ""
    $markdownLines += "## Captures"
    $markdownLines += ""

    foreach ($viewGroup in ($Report.captures | Group-Object -Property view)) {
        $markdownLines += "### $($viewGroup.Name)"
        $markdownLines += ""
        foreach ($capture in ($viewGroup.Group | Sort-Object width, height)) {
            $screenshotRelative = ConvertTo-RelativeReportPath -BasePath $OutputRoot -TargetPath $capture.screenshotPath
            $geometryRelative = ConvertTo-RelativeReportPath -BasePath $OutputRoot -TargetPath $capture.geometryPath
            $markdownLines += "#### $($capture.width)x$($capture.height)"
            $markdownLines += ""
            $markdownLines += "- Warnings: $($capture.warningCount)"
            $markdownLines += "- Geometry: [$geometryRelative]($geometryRelative)"
            $markdownLines += ""
            $markdownLines += "![Sorotte GUI $($capture.view) $($capture.width)x$($capture.height)]($screenshotRelative)"
            $markdownLines += ""
        }
    }
    $markdownLines | Set-Content -LiteralPath $markdownPath -Encoding UTF8

    $htmlLines = @()
    $htmlLines += "<!doctype html>"
    $htmlLines += "<html lang=""en"">"
    $htmlLines += "<head>"
    $htmlLines += "  <meta charset=""utf-8"">"
    $htmlLines += "  <title>Sorotte GUI Positioning Report</title>"
    $htmlLines += "  <style>"
    $htmlLines += "    body { font-family: Segoe UI, Arial, sans-serif; margin: 24px; color: #17212b; background: #f6f8fa; }"
    $htmlLines += "    h1, h2, h3 { margin-bottom: 0.35rem; }"
    $htmlLines += "    table { border-collapse: collapse; width: 100%; background: white; }"
    $htmlLines += "    th, td { border: 1px solid #ced8df; padding: 6px 8px; text-align: left; vertical-align: top; }"
    $htmlLines += "    .capture { background: white; border: 1px solid #ced8df; border-radius: 6px; margin: 16px 0; padding: 12px; }"
    $htmlLines += "    .capture img { max-width: 100%; height: auto; border: 1px solid #ced8df; }"
    $htmlLines += "    .meta { color: #687683; }"
    $htmlLines += "  </style>"
    $htmlLines += "</head>"
    $htmlLines += "<body>"
    $htmlLines += "  <h1>Sorotte GUI Positioning Report</h1>"
    $htmlLines += "  <p class=""meta"">Result: $(ConvertTo-HtmlText $Report.result) | Captures: $($Report.captures.Count) | Warnings: $($Report.warnings.Count) | Generated: $(ConvertTo-HtmlText $Report.generatedAt)</p>"
    $htmlLines += "  <p class=""meta"">Binary: <code>$(ConvertTo-HtmlText $Report.binary)</code></p>"
    $htmlLines += "  <h2>Warnings</h2>"
    if ($Report.warnings.Count -eq 0) {
        $htmlLines += "  <p>No positioning warnings were reported.</p>"
    } else {
        $htmlLines += "  <table>"
        $htmlLines += "    <thead><tr><th>View</th><th>Size</th><th>Code</th><th>Message</th><th>Controls</th></tr></thead>"
        $htmlLines += "    <tbody>"
        foreach ($warning in $Report.warnings) {
            $controlText = ($warning.controls -join ", ")
            $htmlLines += "      <tr><td>$(ConvertTo-HtmlText $warning.view)</td><td>$($warning.width)x$($warning.height)</td><td>$(ConvertTo-HtmlText $warning.code)</td><td>$(ConvertTo-HtmlText $warning.message)</td><td>$(ConvertTo-HtmlText $controlText)</td></tr>"
        }
        $htmlLines += "    </tbody>"
        $htmlLines += "  </table>"
    }
    $htmlLines += "  <h2>Captures</h2>"
    foreach ($viewGroup in ($Report.captures | Group-Object -Property view)) {
        $htmlLines += "  <h3>$(ConvertTo-HtmlText $viewGroup.Name)</h3>"
        foreach ($capture in ($viewGroup.Group | Sort-Object width, height)) {
            $screenshotRelative = (ConvertTo-RelativeReportPath -BasePath $OutputRoot -TargetPath $capture.screenshotPath).Replace('\', '/')
            $geometryRelative = (ConvertTo-RelativeReportPath -BasePath $OutputRoot -TargetPath $capture.geometryPath).Replace('\', '/')
            $htmlLines += "  <section class=""capture"">"
            $htmlLines += "    <h4>$($capture.width)x$($capture.height)</h4>"
            $htmlLines += "    <p class=""meta"">Warnings: $($capture.warningCount) | Geometry: <a href=""$(ConvertTo-HtmlText $geometryRelative)"">$(ConvertTo-HtmlText $geometryRelative)</a></p>"
            $htmlLines += "    <img src=""$(ConvertTo-HtmlText $screenshotRelative)"" alt=""Sorotte GUI $(ConvertTo-HtmlText $capture.view) $($capture.width)x$($capture.height)"">"
            $htmlLines += "  </section>"
        }
    }
    $htmlLines += "</body>"
    $htmlLines += "</html>"
    $htmlLines | Set-Content -LiteralPath $htmlPath -Encoding UTF8

    return [pscustomobject]@{
        markdown = (Resolve-Path -LiteralPath $markdownPath).Path
        html = (Resolve-Path -LiteralPath $htmlPath).Path
    }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$outputRoot = ConvertTo-ReportPath -Path $OutputDir -Root $repoRoot
New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null

if (-not $BinaryPath) {
    $BinaryPath = Join-Path $repoRoot "target\debug\sorotte-gui.exe"
}
$binaryFullPath = ConvertTo-ReportPath -Path $BinaryPath -Root $repoRoot

if (-not $NoBuild) {
    Push-Location $repoRoot
    try {
        & cargo build -p sorotte-gui --bin sorotte-gui
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $binaryFullPath)) {
    throw "sorotte-gui binary does not exist at $binaryFullPath"
}

$captureViews = Resolve-ReportViews -RawViews $View
$captureSizes = Resolve-ReportSizes -RawSizes $Size
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$generatedAt = (Get-Date).ToString("o")
$captures = @()
$allWarnings = @()

foreach ($activeView in $captureViews) {
    foreach ($captureSize in $captureSizes) {
        $seedView = if ($activeView -eq "playlist-urls" -or $activeView -eq "room-change") { "room" } else { $activeView }
        $profileRoot = Join-Path $repoRoot ("target\gui-positioning-runtime\{0}-{1}-{2}-{3}" -f $timestamp, $PID, $activeView, $captureSize.key)
        $configPath = Write-PositioningConfig -Root $profileRoot -ActiveView $seedView
        $process = $null
        $hwnd = [IntPtr]::Zero

        try {
            $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
            $startInfo.FileName = $binaryFullPath
            $startInfo.WorkingDirectory = (Split-Path -Parent $binaryFullPath)
            $startInfo.UseShellExecute = $false
            $startInfo.Environment["SOROTTE_CLIENT_CONFIG_PATH"] = $configPath
            $startInfo.Environment["SOROTTE_GUI_ENABLE_TEST_PLAYER"] = "true"
            if ($activeView -eq "room" -or $activeView -eq "room-change" -or $activeView -eq "playlist-urls") {
                $startInfo.Environment["SOROTTE_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK"] = "true"
                $startInfo.Environment["SOROTTE_CLIENT_USERNAME"] = "smoke-user"
                $startInfo.Environment["SOROTTE_CLIENT_ROOM"] = "smoke-room"
            }
            $startInfo.Environment["SOROTTE_GUI_REFRESH_PUBLIC_SERVERS"] = "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]"
            $startInfo.Environment["SOROTTE_GUI_UPDATE_CHECK_RESPONSE"] = '{"version-status":"uptodate","version-message":"Sorotte is up to date."}'
            $startInfo.Environment["SOROTTE_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH"] = Join-Path $profileRoot "media-search"
            $startInfo.Environment["SOROTTE_GUI_TEST_OPEN_MEDIA_FILE_PATHS"] = Join-Path $profileRoot "open-target.mkv"
            New-Item -ItemType Directory -Force -Path $startInfo.Environment["SOROTTE_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH"] | Out-Null
            Set-Content -LiteralPath $startInfo.Environment["SOROTTE_GUI_TEST_OPEN_MEDIA_FILE_PATHS"] -Value "open-target" -Encoding ASCII

            $process = [System.Diagnostics.Process]::Start($startInfo)
            $hwnd = [SorottePositioningProbe]::FindMainWindowForProcess($process.Id, $TimeoutMs)
            if ($hwnd -eq [IntPtr]::Zero) {
                throw "timed out waiting for Sorotte GUI window for pid $($process.Id)"
            }

            $bounds = [SorottePositioningProbe]::PrepareWindow($hwnd, $WindowX, $WindowY, $captureSize.width, $captureSize.height)
            Invoke-PositioningViewSetup -Window $hwnd -ActiveView $activeView -SampleUrls $SampleUrls
            Start-Sleep -Milliseconds 900

            $screenshotPath = Join-Path $outputRoot ("sorotte-gui-{0}-{1}-{2}.png" -f $activeView, $captureSize.key, $timestamp)
            $geometryPath = Join-Path $outputRoot ("sorotte-gui-{0}-{1}-{2}.geometry.json" -f $activeView, $captureSize.key, $timestamp)
            [SorottePositioningProbe]::CaptureWindow($hwnd, $screenshotPath)
            $controls = [SorottePositioningProbe]::CollectControls($hwnd)
            $win32WindowBounds = [pscustomobject]@{
                left = $bounds.Left
                top = $bounds.Top
                right = $bounds.Right
                bottom = $bounds.Bottom
                width = ($bounds.Right - $bounds.Left)
                height = ($bounds.Bottom - $bounds.Top)
            }
            $rootControl = @($controls | Where-Object { $_.Depth -eq 0 } | Select-Object -First 1)
            if ($rootControl.Count -gt 0 -and [double]$rootControl[0].Width -gt 0 -and [double]$rootControl[0].Height -gt 0) {
                $uiaWindowBounds = [pscustomobject]@{
                    left = [double]$rootControl[0].Left
                    top = [double]$rootControl[0].Top
                    right = [double]$rootControl[0].Right
                    bottom = [double]$rootControl[0].Bottom
                    width = [double]$rootControl[0].Width
                    height = [double]$rootControl[0].Height
                }
            } else {
                $uiaWindowBounds = $win32WindowBounds
            }
            $captureWarnings = @(Get-PositioningWarnings `
                -ActiveView $activeView `
                -RequestedWidth $captureSize.width `
                -RequestedHeight $captureSize.height `
                -WindowBounds $uiaWindowBounds `
                -Controls $controls)

            $geometryReport = [pscustomobject]@{
                view = $activeView
                requestedWidth = $captureSize.width
                requestedHeight = $captureSize.height
                theme = $defaultThemeLabel
                state = "deterministic-rust"
                capturedAt = (Get-Date).ToString("o")
                windowBounds = $win32WindowBounds
                uiaWindowBounds = $uiaWindowBounds
                controls = $controls
                warnings = $captureWarnings
            }
            $geometryReport | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $geometryPath -Encoding UTF8

            $screenshotFullPath = (Resolve-Path -LiteralPath $screenshotPath).Path
            $geometryFullPath = (Resolve-Path -LiteralPath $geometryPath).Path
            $captures += [pscustomobject]@{
                view = $activeView
                width = $captureSize.width
                height = $captureSize.height
                theme = $defaultThemeLabel
                state = "deterministic-rust"
                pid = $process.Id
                screenshotPath = $screenshotFullPath
                geometryPath = $geometryFullPath
                warningCount = $captureWarnings.Count
                windowBounds = $win32WindowBounds
                uiaWindowBounds = $uiaWindowBounds
            }
            foreach ($warning in $captureWarnings) {
                $allWarnings += $warning
            }
        } finally {
            if ($hwnd -ne [IntPtr]::Zero) {
                try {
                    [SorottePositioningProbe]::ClearTopmost($hwnd)
                } catch {
                }
            }
            if (-not $KeepOpen -and $hwnd -ne [IntPtr]::Zero) {
                [SorottePositioningProbe]::CloseWindow($hwnd)
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
}

$result = if ($WarningsAsErrors -and $allWarnings.Count -gt 0) { "warning-failed" } else { "ok" }
$manifestPath = Join-Path $outputRoot ("manifest-{0}.json" -f $timestamp)
$report = [pscustomobject]@{
    result = $result
    binary = (Resolve-Path -LiteralPath $binaryFullPath).Path
    outputDir = (Resolve-Path -LiteralPath $outputRoot).Path
    generatedAt = $generatedAt
    captures = $captures
    warnings = $allWarnings
}
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
$indexes = Write-ReportIndexes -OutputRoot $outputRoot -Report $report
$report | Add-Member -NotePropertyName manifestPath -NotePropertyValue (Resolve-Path -LiteralPath $manifestPath).Path
$report | Add-Member -NotePropertyName indexMarkdownPath -NotePropertyValue $indexes.markdown
$report | Add-Member -NotePropertyName indexHtmlPath -NotePropertyValue $indexes.html

if ($Json) {
    $report | ConvertTo-Json -Depth 8
} else {
    Write-Output "result=$($report.result)"
    Write-Output "binary=$($report.binary)"
    Write-Output "outputDir=$($report.outputDir)"
    Write-Output "captures=$($report.captures.Count)"
    Write-Output "warnings=$($report.warnings.Count)"
    Write-Output "manifest=$($report.manifestPath)"
    Write-Output "indexMarkdown=$($report.indexMarkdownPath)"
    Write-Output "indexHtml=$($report.indexHtmlPath)"
}

if ($WarningsAsErrors -and $allWarnings.Count -gt 0) {
    exit 1
}
