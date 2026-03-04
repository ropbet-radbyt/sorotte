param(
    [string]$BinaryPath = (Join-Path $PSScriptRoot "..\target\debug\syncplay-gui.exe"),
    [string]$TempRoot = (Join-Path $env:TEMP "syncplay-gui-smoke-regression")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$script:SavedEnv = $null

Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class GuiSmokeNative {
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
}
"@

function Assert-True {
    param(
        [bool]$Condition,
        [string]$Message
    )

    if (-not $Condition) {
        throw $Message
    }
}

function Get-ButtonCondition {
    param([string]$Name)

    return [System.Windows.Automation.AndCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::NameProperty,
            $Name
        ),
        [System.Windows.Automation.PropertyCondition]::new(
            [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
            [System.Windows.Automation.ControlType]::Button
        )
    )
}

function Wait-GuiWindow {
    param(
        [System.Diagnostics.Process]$Process,
        [int]$TimeoutMs = 10000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Process.Refresh()
        if ($Process.HasExited) {
            $exitCode = [uint32]$Process.ExitCode
            throw ('syncplay-gui exited before creating a window (exit code 0x{0:X8}).' -f $exitCode)
        }
        if ($Process.MainWindowHandle -ne 0) {
            $root = [System.Windows.Automation.AutomationElement]::FromHandle(
                [IntPtr]$Process.MainWindowHandle
            )
            if ($null -ne $root) {
                $names = Get-AllTextNames -Root $root
                if (($names -contains "File") -and ($names | Where-Object { $_ -like "view: *" })) {
                    return $root
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Timed out waiting for the syncplay-gui main window."
}

function Open-SmokeWindow {
    param(
        [string]$ResolvedBinaryPath,
        [string]$ConfigPath,
        [string]$MediaSearchBrowsePath,
        [string]$OpenMediaFilePath,
        [bool]$EnableLoopbackSession = $false,
        [bool]$EnableTcpSession = $false,
        [int]$TcpSessionPort = 0,
        [string]$PublicServersSpec = "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]",
        [int]$MaxAttempts = 2
    )

    $lastError = $null

    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        $process = $null
        try {
            $process = Start-SmokeProcess -ResolvedBinaryPath $ResolvedBinaryPath -ConfigPath $ConfigPath -MediaSearchBrowsePath $MediaSearchBrowsePath -OpenMediaFilePath $OpenMediaFilePath -EnableLoopbackSession $EnableLoopbackSession -EnableTcpSession $EnableTcpSession -TcpSessionPort $TcpSessionPort -PublicServersSpec $PublicServersSpec
            $root = Wait-GuiWindow -Process $process
            return [pscustomobject]@{
                Process = $process
                Root = $root
            }
        }
        catch {
            $lastError = $_
            $isDllInitFailure = $false
            if ($null -ne $process) {
                $process.Refresh()
                if ($process.HasExited) {
                    $isDllInitFailure = ([uint32]$process.ExitCode) -eq 0xC0000142
                } else {
                    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
                    $process.WaitForExit()
                }
            }

            if ($isDllInitFailure -and $attempt -lt $MaxAttempts) {
                Start-Sleep -Milliseconds 500
                continue
            }

            throw
        }
    }

    throw $lastError
}

function Find-Buttons {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name
    )

    return ,$Root.FindAll(
        [System.Windows.Automation.TreeScope]::Descendants,
        (Get-ButtonCondition -Name $Name)
    )
}

function Find-Edits {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name = $null
    )

    $typeCondition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Edit
    )
    $condition = $typeCondition
    if (-not [string]::IsNullOrEmpty($Name)) {
        $condition = [System.Windows.Automation.AndCondition]::new(
            $typeCondition,
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::NameProperty,
                $Name
            )
        )
    }

    return ,$Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
}

function Find-FirstEdit {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name,
        [int]$Index = 0
    )

    $edits = Find-Edits -Root $Root -Name $Name
    if ($edits.Count -le $Index) {
        throw "Edit not found: $Name [$Index]"
    }
    return $edits.Item($Index)
}

function Find-FirstButton {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name,
        [int]$Index = 0
    )

    $buttons = Find-Buttons -Root $Root -Name $Name
    if ($buttons.Count -le $Index) {
        throw "Button not found: $Name [$Index]"
    }
    return $buttons.Item($Index)
}

function Invoke-Button {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name,
        [int]$Index = 0
    )

    $button = Find-FirstButton -Root $Root -Name $Name -Index $Index
    $pattern = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $pattern.Invoke()
    Start-Sleep -Milliseconds 250
}

function Get-EditElements {
    param([System.Windows.Automation.AutomationElement]$Root)

    return ,(Find-Edits -Root $Root)
}

function Get-EditValue {
    param([System.Windows.Automation.AutomationElement]$Edit)

    $pattern = $Edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern)
    return $pattern.Current.Value
}

function Set-EditValue {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [System.Windows.Automation.AutomationElement]$Edit,
        [string]$Value
    )

    $window = $Root.Current.NativeWindowHandle
    [void][GuiSmokeNative]::SetForegroundWindow([IntPtr]$window)
    Start-Sleep -Milliseconds 150
    $Edit.SetFocus()
    Start-Sleep -Milliseconds 100
    [System.Windows.Forms.SendKeys]::SendWait("^a")
    Start-Sleep -Milliseconds 50
    [System.Windows.Forms.SendKeys]::SendWait("{BACKSPACE}")
    Start-Sleep -Milliseconds 50
    if (-not [string]::IsNullOrEmpty($Value)) {
        [System.Windows.Forms.SendKeys]::SendWait($Value)
        Start-Sleep -Milliseconds 100
    }
}

function Set-EditValueAndAssert {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [System.Windows.Automation.AutomationElement]$Edit,
        [string]$Value,
        [int]$Attempts = 3
    )

    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        Set-EditValue -Root $Root -Edit $Edit -Value $Value
        if ((Get-EditValue -Edit $Edit) -eq $Value) {
            return
        }
        Start-Sleep -Milliseconds 150
    }

    throw "Failed to set edit value after $Attempts attempts. Expected '$Value', got '$(Get-EditValue -Edit $Edit)'."
}

function Submit-EditValue {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [System.Windows.Automation.AutomationElement]$Edit,
        [string]$Value
    )

    Set-EditValueAndAssert -Root $Root -Edit $Edit -Value $Value
    $window = $Root.Current.NativeWindowHandle
    [void][GuiSmokeNative]::SetForegroundWindow([IntPtr]$window)
    Start-Sleep -Milliseconds 150
    $Edit.SetFocus()
    Start-Sleep -Milliseconds 100
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
    Start-Sleep -Milliseconds 250
}

function Get-FreeTcpPort {
    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
    $listener.Start()
    try {
        return ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
    }
    finally {
        $listener.Stop()
    }
}

function Start-MockSyncplayTcpServer {
    param(
        [string]$TempRootPath,
        [string]$Username,
        [string]$Room,
        [string]$Name = "mock-syncplay-server",
        [string[]]$PlaylistFiles = @("episode1.mkv", "episode2.mkv"),
        [int]$PlaylistIndex = 1,
        [bool]$Ready = $true,
        [bool]$Paused = $true,
        [bool]$SendPostChatState = $false,
        [string[]]$PostChatPlaylistFiles = @("postchat1.mkv", "postchat2.mkv"),
        [int]$PostChatPlaylistIndex = 1,
        [bool]$PostChatReady = $false,
        [bool]$PostChatPaused = $false,
        [string]$RemoteUsername = "bob",
        [bool]$RemoteUserReady = $true,
        [bool]$RemoteUserController = $true,
        [string]$RemoteUserFile = "bob.mp4",
        [string]$PostChatRemoteUsername = "",
        [bool]$PostChatRemoteUserReady = $false,
        [bool]$PostChatRemoteUserController = $false,
        [string]$PostChatRemoteUserFile = "bob-post.mp4",
        [bool]$SendUserLeftOnSecondChat = $false,
        [string]$UserLeftUsername = ""
    )

    $port = Get-FreeTcpPort
    $readyPath = Join-Path $TempRootPath "$Name.ready"
    $logPath = Join-Path $TempRootPath "$Name.log"
    if ([string]::IsNullOrWhiteSpace($PostChatRemoteUsername)) {
        $PostChatRemoteUsername = $RemoteUsername
    }
    if ([string]::IsNullOrWhiteSpace($UserLeftUsername)) {
        $UserLeftUsername = $PostChatRemoteUsername
    }
    $playlistFilesJson = $PlaylistFiles | ConvertTo-Json -Compress
    $readyJson = $Ready | ConvertTo-Json -Compress
    $pausedJson = $Paused | ConvertTo-Json -Compress
    $sendPostChatStateJson = $SendPostChatState | ConvertTo-Json -Compress
    $postChatPlaylistFilesJson = $PostChatPlaylistFiles | ConvertTo-Json -Compress
    $postChatReadyJson = $PostChatReady | ConvertTo-Json -Compress
    $postChatPausedJson = $PostChatPaused | ConvertTo-Json -Compress
    $remoteUsernameJson = $RemoteUsername | ConvertTo-Json -Compress
    $remoteUserReadyJson = $RemoteUserReady | ConvertTo-Json -Compress
    $remoteUserControllerJson = $RemoteUserController | ConvertTo-Json -Compress
    $remoteUserFileJson = $RemoteUserFile | ConvertTo-Json -Compress
    $postChatRemoteUsernameJson = $PostChatRemoteUsername | ConvertTo-Json -Compress
    $postChatRemoteUserReadyJson = $PostChatRemoteUserReady | ConvertTo-Json -Compress
    $postChatRemoteUserControllerJson = $PostChatRemoteUserController | ConvertTo-Json -Compress
    $postChatRemoteUserFileJson = $PostChatRemoteUserFile | ConvertTo-Json -Compress
    $sendUserLeftOnSecondChatJson = $SendUserLeftOnSecondChat | ConvertTo-Json -Compress
    $userLeftUsernameJson = $UserLeftUsername | ConvertTo-Json -Compress
    Remove-Item $readyPath -ErrorAction SilentlyContinue
    Remove-Item $logPath -ErrorAction SilentlyContinue

    $job = Start-Job -ScriptBlock {
        param($Port, $ReadyPath, $LogPath, $Username, $Room, $PlaylistFilesJson, $PlaylistIndex, $ReadyJson, $PausedJson, $SendPostChatStateJson, $PostChatPlaylistFilesJson, $PostChatPlaylistIndex, $PostChatReadyJson, $PostChatPausedJson, $RemoteUsernameJson, $RemoteUserReadyJson, $RemoteUserControllerJson, $RemoteUserFileJson, $PostChatRemoteUsernameJson, $PostChatRemoteUserReadyJson, $PostChatRemoteUserControllerJson, $PostChatRemoteUserFileJson, $SendUserLeftOnSecondChatJson, $UserLeftUsernameJson)

        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, [int]$Port)
        $listener.Start()
        Set-Content -Path $ReadyPath -Value "ready"

        try {
            $client = $listener.AcceptTcpClient()
            try {
                $sendPostChatState = [bool]::Parse($SendPostChatStateJson)
                $sendUserLeftOnSecondChat = [bool]::Parse($SendUserLeftOnSecondChatJson)
                $postChatStateSent = $false
                $userLeftSent = $false
                $chatCount = 0
                $stream = $client.GetStream()
                $reader = [System.IO.StreamReader]::new($stream)
                $writer = [System.IO.StreamWriter]::new($stream)
                $writer.AutoFlush = $true

                while ($true) {
                    $line = $reader.ReadLine()
                    if ($null -eq $line) {
                        break
                    }

                    Add-Content -Path $LogPath -Value $line

                    try {
                        $payload = $line | ConvertFrom-Json -ErrorAction Stop
                    }
                    catch {
                        continue
                    }

                    if ($null -ne $payload.Hello) {
                        $writer.WriteLine(
                            ('{{"Hello":{{"username":{0},"room":{{"name":{1}}},"version":"1.7.5","features":{{"chat":true}}}}}}' -f (
                                $Username | ConvertTo-Json -Compress
                            ), (
                                $Room | ConvertTo-Json -Compress
                            ))
                        )
                        $writer.WriteLine(
                            ('{{"Set":{{"playlistChange":{{"files":{0},"user":"smoke-user"}}}}}}' -f $PlaylistFilesJson)
                        )
                        $writer.WriteLine(
                            ('{{"Set":{{"playlistIndex":{{"index":{0},"user":"smoke-user"}}}}}}' -f $PlaylistIndex)
                        )
                        $writer.WriteLine(
                            ('{{"Set":{{"ready":{{"isReady":{0},"username":"smoke-user"}}}}}}' -f $ReadyJson)
                        )
                        $writer.WriteLine(
                            ('{{"State":{{"playstate":{{"position":10.0,"paused":{0},"doSeek":false,"setBy":"smoke-user"}}}}}}' -f $PausedJson)
                        )
                        $writer.WriteLine(
                            ('{"Set":{"user":{' +
                                $RemoteUsernameJson +
                                ':{"room":{"name":' +
                                ($Room | ConvertTo-Json -Compress) +
                                '},"file":{"name":' +
                                $RemoteUserFileJson +
                                '},"isReady":' +
                                $RemoteUserReadyJson +
                                ',"controller":' +
                                $RemoteUserControllerJson +
                                '}}}}')
                        )
                        continue
                    }

                    if ($null -eq $payload.Chat) {
                        continue
                    }

                    $message = $null
                    if ($payload.Chat -is [string]) {
                        $message = [string]$payload.Chat
                    } elseif ($null -ne $payload.Chat.message) {
                        $message = [string]$payload.Chat.message
                    }

                    if (-not [string]::IsNullOrEmpty($message)) {
                        $chatCount += 1
                        $writer.WriteLine(
                            ('{{"Chat":{{"username":{0},"message":{1}}}}}' -f (
                                $Username | ConvertTo-Json -Compress
                            ), (
                                $message | ConvertTo-Json -Compress
                            ))
                        )
                        if ($sendPostChatState -and -not $postChatStateSent) {
                            $writer.WriteLine(
                                ('{{"Set":{{"playlistChange":{{"files":{0},"user":"smoke-user"}}}}}}' -f $PostChatPlaylistFilesJson)
                            )
                            $writer.WriteLine(
                                ('{{"Set":{{"playlistIndex":{{"index":{0},"user":"smoke-user"}}}}}}' -f $PostChatPlaylistIndex)
                            )
                            $writer.WriteLine(
                                ('{{"Set":{{"ready":{{"isReady":{0},"username":"smoke-user"}}}}}}' -f $PostChatReadyJson)
                            )
                            $writer.WriteLine(
                                ('{{"State":{{"playstate":{{"position":20.0,"paused":{0},"doSeek":false,"setBy":"smoke-user"}}}}}}' -f $PostChatPausedJson)
                            )
                            $writer.WriteLine(
                                ('{"Set":{"user":{' +
                                    $PostChatRemoteUsernameJson +
                                    ':{"room":{"name":' +
                                    ($Room | ConvertTo-Json -Compress) +
                                    '},"file":{"name":' +
                                    $PostChatRemoteUserFileJson +
                                    '},"isReady":' +
                                    $PostChatRemoteUserReadyJson +
                                    ',"controller":' +
                                    $PostChatRemoteUserControllerJson +
                                    '}}}}')
                            )
                            $postChatStateSent = $true
                        }
                        if ($sendUserLeftOnSecondChat -and -not $userLeftSent -and $chatCount -ge 2) {
                            $writer.WriteLine(
                                ('{"Set":{"user":{' +
                                    $UserLeftUsernameJson +
                                    ':{"event":{"left":true}}}}}')
                            )
                            $userLeftSent = $true
                        }
                    }
                }
            }
            finally {
                if ($null -ne $client) {
                    $client.Dispose()
                }
            }
        }
        finally {
            $listener.Stop()
        }
    } -ArgumentList $port, $readyPath, $logPath, $Username, $Room, $playlistFilesJson, $PlaylistIndex, $readyJson, $pausedJson, $sendPostChatStateJson, $postChatPlaylistFilesJson, $PostChatPlaylistIndex, $postChatReadyJson, $postChatPausedJson, $remoteUsernameJson, $remoteUserReadyJson, $remoteUserControllerJson, $remoteUserFileJson, $postChatRemoteUsernameJson, $postChatRemoteUserReadyJson, $postChatRemoteUserControllerJson, $postChatRemoteUserFileJson, $sendUserLeftOnSecondChatJson, $userLeftUsernameJson

    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        if (Test-Path $readyPath) {
            return [pscustomobject]@{
                Port = $port
                ReadyPath = $readyPath
                LogPath = $logPath
                Job = $job
            }
        }
        if ($job.State -match "Completed|Failed|Stopped") {
            $jobOutput = (Receive-Job -Job $job -Keep -ErrorAction SilentlyContinue | Out-String).Trim()
            throw "Mock Syncplay TCP server exited before becoming ready. $jobOutput"
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Timed out waiting for the mock Syncplay TCP server to become ready."
}

function Stop-MockSyncplayTcpServer {
    param($Server)

    if ($null -eq $Server) {
        return
    }

    if ($null -ne $Server.Job) {
        if ($Server.Job.State -eq "Running") {
            Stop-Job -Job $Server.Job -ErrorAction SilentlyContinue | Out-Null
        }
        Receive-Job -Job $Server.Job -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $Server.Job -Force -ErrorAction SilentlyContinue
    }

    if ($null -ne $Server.ReadyPath) {
        Remove-Item $Server.ReadyPath -ErrorAction SilentlyContinue
    }
}

function Find-MainWindowChatInputEdit {
    param([System.Windows.Automation.AutomationElement]$Root)

    $namedEdits = Find-Edits -Root $Root -Name "Chat Input"
    if ($namedEdits.Count -gt 0) {
        return $namedEdits.Item(0)
    }

    $edits = Get-EditElements -Root $Root
    Assert-True ($edits.Count -gt 0) "Expected at least one edit field on the main window."
    return $edits.Item($edits.Count - 1)
}

function Get-AllTextNames {
    param([System.Windows.Automation.AutomationElement]$Root)

    $elements = $Root.FindAll(
        [System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition
    )
    $names = @()
    for ($i = 0; $i -lt $elements.Count; $i++) {
        $element = $elements.Item($i)
        $name = $element.Current.Name
        if (-not [string]::IsNullOrWhiteSpace($name)) {
            $names += $name
        }
    }
    return $names
}

function Assert-ContainsName {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Expected,
        [int]$TimeoutMs = 5000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $names = Get-AllTextNames -Root $Root
        if ($names -contains $Expected) {
            return
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Expected UI text not found: $Expected"
}

function Assert-StatusValue {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Label,
        [string]$ExpectedValue,
        [int]$TimeoutMs = 5000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $names = Get-AllTextNames -Root $Root
        for ($i = 0; $i -lt $names.Count; $i++) {
            if ($names[$i] -ne $Label) {
                continue
            }
            $windowEnd = [Math]::Min($names.Count - 1, $i + 3)
            for ($j = $i + 1; $j -le $windowEnd; $j++) {
                if ($names[$j] -eq $ExpectedValue) {
                    return
                }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Expected status value not found: $Label -> $ExpectedValue"
}

function Wait-ForMainWindowSessionState {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$ExpectedPlaylistItem,
        [string]$ExpectedUserRow,
        [string]$ExpectedPausedValue,
        [string[]]$ExpectedAdditionalNames = @(),
        [string[]]$UnexpectedNames = @(),
        [int]$TimeoutMs = 8000
    )

    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $lastError = $null

    do {
        try {
            Invoke-Button -Root $Root -Name "Main Window"
            $names = Get-AllTextNames -Root $Root
            Assert-True ($names -contains "view: main-window") "Expected main window view indicator."
            Assert-True ($names -contains $ExpectedPlaylistItem) "Expected playlist item not found: $ExpectedPlaylistItem"
            Assert-True ($names -contains $ExpectedUserRow) "Expected primary user row not found: $ExpectedUserRow"
            foreach ($expectedName in $ExpectedAdditionalNames) {
                Assert-True ($names -contains $expectedName) "Expected UI text not found: $expectedName"
            }
            foreach ($unexpectedName in $UnexpectedNames) {
                Assert-True (-not ($names -contains $unexpectedName)) "Unexpected UI text still present: $unexpectedName"
            }
            $statusNames = $names
            $foundStatusValue = $false
            for ($i = 0; $i -lt $statusNames.Count; $i++) {
                if ($statusNames[$i] -ne "Playback Paused") {
                    continue
                }
                $windowEnd = [Math]::Min($statusNames.Count - 1, $i + 3)
                for ($j = $i + 1; $j -le $windowEnd; $j++) {
                    if ($statusNames[$j] -eq $ExpectedPausedValue) {
                        $foundStatusValue = $true
                        break
                    }
                }
                if ($foundStatusValue) {
                    break
                }
            }
            Assert-True $foundStatusValue "Expected status value not found: Playback Paused -> $ExpectedPausedValue"
            return
        }
        catch {
            $lastError = $_
            Start-Sleep -Milliseconds 150
        }
    } while ([DateTime]::UtcNow -lt $deadline)

    if ($null -ne $lastError) {
        throw $lastError
    }

    throw "Timed out waiting for the expected main-window session state."
}

function Start-SmokeProcess {
    param(
        [string]$ResolvedBinaryPath,
        [string]$ConfigPath,
        [string]$MediaSearchBrowsePath,
        [string]$OpenMediaFilePath,
        [bool]$EnableLoopbackSession = $false,
        [bool]$EnableTcpSession = $false,
        [int]$TcpSessionPort = 0,
        [string]$PublicServersSpec = "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]"
    )

    if ($null -eq $script:SavedEnv) {
        $script:SavedEnv = @{
            SYNCPLAY_CLIENT_CONFIG_PATH = $env:SYNCPLAY_CLIENT_CONFIG_PATH
            SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP = $env:SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP
            SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK = $env:SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK
            SYNCPLAY_CLIENT_HOST = $env:SYNCPLAY_CLIENT_HOST
            SYNCPLAY_CLIENT_PORT = $env:SYNCPLAY_CLIENT_PORT
            SYNCPLAY_CLIENT_USERNAME = $env:SYNCPLAY_CLIENT_USERNAME
            SYNCPLAY_CLIENT_NAME = $env:SYNCPLAY_CLIENT_NAME
            SYNCPLAY_CLIENT_ROOM = $env:SYNCPLAY_CLIENT_ROOM
            SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS = $env:SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS
            SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH = $env:SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH
            SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS = $env:SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS
        }
    }

    $env:SYNCPLAY_CLIENT_CONFIG_PATH = $ConfigPath
    Remove-Item "Env:SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP" -ErrorAction SilentlyContinue
    Remove-Item "Env:SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK" -ErrorAction SilentlyContinue
    Remove-Item "Env:SYNCPLAY_CLIENT_HOST" -ErrorAction SilentlyContinue
    Remove-Item "Env:SYNCPLAY_CLIENT_PORT" -ErrorAction SilentlyContinue
    Remove-Item "Env:SYNCPLAY_CLIENT_USERNAME" -ErrorAction SilentlyContinue
    Remove-Item "Env:SYNCPLAY_CLIENT_NAME" -ErrorAction SilentlyContinue
    Remove-Item "Env:SYNCPLAY_CLIENT_ROOM" -ErrorAction SilentlyContinue
    $env:SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS = $PublicServersSpec
    $env:SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH = $MediaSearchBrowsePath
    $env:SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS = $OpenMediaFilePath
    if ($EnableTcpSession) {
        Assert-True ($TcpSessionPort -gt 0) "TCP session mode requires a positive port."
        $env:SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP = "true"
        $env:SYNCPLAY_CLIENT_HOST = "127.0.0.1"
        $env:SYNCPLAY_CLIENT_PORT = [string]$TcpSessionPort
        $env:SYNCPLAY_CLIENT_USERNAME = "smoke-user"
        $env:SYNCPLAY_CLIENT_ROOM = "smoke-room"
    } elseif ($EnableLoopbackSession) {
        $env:SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK = "true"
        $env:SYNCPLAY_CLIENT_USERNAME = "smoke-user"
        $env:SYNCPLAY_CLIENT_ROOM = "smoke-room"
    }

    $workingDirectory = Split-Path -Parent $ResolvedBinaryPath
    return Start-Process -FilePath $ResolvedBinaryPath -WorkingDirectory $workingDirectory -PassThru
}

function Restore-SmokeEnvironment {
    if ($null -eq $script:SavedEnv) {
        return
    }

    foreach ($entry in $script:SavedEnv.GetEnumerator()) {
        if ($null -eq $entry.Value) {
            Remove-Item "Env:$($entry.Key)" -ErrorAction SilentlyContinue
        } else {
            Set-Item "Env:$($entry.Key)" $entry.Value
        }
    }
}

$resolvedBinaryPath = [System.IO.Path]::GetFullPath($BinaryPath)
Assert-True (Test-Path $resolvedBinaryPath) "Binary not found: $resolvedBinaryPath"

$tempRootPath = [System.IO.Path]::GetFullPath($TempRoot)
New-Item -ItemType Directory -Force -Path $tempRootPath | Out-Null
$configPath = Join-Path $tempRootPath "syncplay.ini"
$mediaSearchDirectoryPath = Join-Path $tempRootPath "media-search"
New-Item -ItemType Directory -Force -Path $mediaSearchDirectoryPath | Out-Null
$openMediaFilePath = Join-Path $tempRootPath "open-target.mkv"
$mediaSearchSampleFilePath = Join-Path $mediaSearchDirectoryPath "search-target.mkv"
Set-Content -Path $openMediaFilePath -Value "open-target"
Set-Content -Path $mediaSearchSampleFilePath -Value "search-target"
if (Test-Path $configPath) {
    Remove-Item $configPath -Force
}

$process = $null
$mockTcpServer = $null
$mockTcpReconnectServer = $null

try {
    $launch = Open-SmokeWindow -ResolvedBinaryPath $resolvedBinaryPath -ConfigPath $configPath -MediaSearchBrowsePath $mediaSearchDirectoryPath -OpenMediaFilePath $openMediaFilePath
    $process = $launch.Process
    $root = $launch.Root

    Assert-ContainsName -Root $root -Expected "view: configuration"
    Assert-ContainsName -Root $root -Expected "Public Servers"
    Assert-ContainsName -Root $root -Expected "2"

    Invoke-Button -Root $root -Name "Help"
    Invoke-Button -Root $root -Name "About"
    Assert-ContainsName -Root $root -Expected "About Syncplay"
    Invoke-Button -Root $root -Name "Close"
    Assert-ContainsName -Root $root -Expected "modal: (none)"

    $edits = Get-EditElements -Root $root
    Assert-True ($edits.Count -ge 6) "Expected at least 6 configuration edit fields."

    $values = @(
        "syncplay.example",
        "8999",
        "smoke-user",
        "smoke-room",
        "",
        "C:\Windows\System32\notepad.exe"
    )
    for ($i = 0; $i -lt $values.Length; $i++) {
        if ($i -eq 4) {
            Set-EditValue -Root $root -Edit $edits.Item($i) -Value $values[$i]
        } else {
            Set-EditValueAndAssert -Root $root -Edit $edits.Item($i) -Value $values[$i]
        }
    }

    Invoke-Button -Root $root -Name "Save"
    Assert-ContainsName -Root $root -Expected "pending: save-configuration"
    Invoke-Button -Root $root -Name "Complete"
    Assert-ContainsName -Root $root -Expected "success: Configuration saved."

    Assert-True (Test-Path $configPath) "Config file was not written: $configPath"
    $configContents = Get-Content $configPath -Raw
    foreach ($expectedLine in @(
        "host = syncplay.example",
        "port = 8999",
        "name = smoke-user",
        "room = smoke-room",
        "playerPath = C:\Windows\System32\notepad.exe"
    )) {
        Assert-True ($configContents.Contains($expectedLine)) "Missing config line: $expectedLine"
    }

    Invoke-Button -Root $root -Name "Public Servers"
    Assert-ContainsName -Root $root -Expected "view: public-servers"
    Invoke-Button -Root $root -Name "Alpha: alpha.example:8999"
    Invoke-Button -Root $root -Name "Connect"
    Assert-ContainsName -Root $root -Expected "pending: connect-public-server"
    Invoke-Button -Root $root -Name "Complete"
    Assert-ContainsName -Root $root -Expected "error: Public server connect requires a session runtime connection; the selected server was not contacted."

    Invoke-Button -Root $root -Name "Refresh"
    Assert-ContainsName -Root $root -Expected "pending: refresh-public-servers"
    Invoke-Button -Root $root -Name "Complete"
    Assert-ContainsName -Root $root -Expected "error: Public server refresh requires a session runtime connection; the server list was not refreshed."

    Invoke-Button -Root $root -Name "Media Search"
    Assert-ContainsName -Root $root -Expected "view: media-search"
    Assert-ContainsName -Root $root -Expected "Browse Directories"
    Assert-ContainsName -Root $root -Expected "Search Missing Media"
    Invoke-Button -Root $root -Name "Browse Directories"
    Assert-ContainsName -Root $root -Expected $mediaSearchDirectoryPath
    Assert-ContainsName -Root $root -Expected "success: Media search directory added: $mediaSearchDirectoryPath."
    Invoke-Button -Root $root -Name "Browse Directories"
    $mediaSearchDirectoryRows = Find-Buttons -Root $root -Name $mediaSearchDirectoryPath
    Assert-True (
        $mediaSearchDirectoryRows.Count -eq 1
    ) "Duplicate media-search browse unexpectedly created extra directory rows."
    Invoke-Button -Root $root -Name "Search Missing Media"
    Assert-ContainsName -Root $root -Expected "pending: search-missing-media"
    Invoke-Button -Root $root -Name "Complete"
    Assert-ContainsName -Root $root -Expected "error: Missing-media search requires a session runtime connection; no search was performed."

    Invoke-Button -Root $root -Name "File"
    Invoke-Button -Root $root -Name "Exit"
    Start-Sleep -Milliseconds 500
    $process.Refresh()
    Assert-True ($process.HasExited) "File -> Exit did not close syncplay-gui."

    $configContents = Get-Content $configPath -Raw
    if ($configContents -match "(?m)^sharedPlaylistEnabled = ") {
        $configContents = [System.Text.RegularExpressions.Regex]::Replace(
            $configContents,
            "(?m)^sharedPlaylistEnabled = .*$",
            "sharedPlaylistEnabled = True"
        )
    } else {
        $configContents = $configContents.TrimEnd() + "`r`nsharedPlaylistEnabled = True`r`n"
    }
    Set-Content -Path $configPath -Value $configContents

    $launch = Open-SmokeWindow -ResolvedBinaryPath $resolvedBinaryPath -ConfigPath $configPath -MediaSearchBrowsePath $mediaSearchDirectoryPath -OpenMediaFilePath $openMediaFilePath
    $process = $launch.Process
    $root = $launch.Root
    Assert-ContainsName -Root $root -Expected "view: configuration"

    $reloadedEdits = Get-EditElements -Root $root
    Assert-True ($reloadedEdits.Count -ge 6) "Expected at least 6 configuration edit fields after relaunch."

    $expectedReloadValues = @{
        0 = "syncplay.example"
        1 = "8999"
        2 = "smoke-user"
        3 = "smoke-room"
        5 = "C:\Windows\System32\notepad.exe"
    }
    foreach ($index in $expectedReloadValues.Keys) {
        $actual = Get-EditValue -Edit $reloadedEdits.Item([int]$index)
        $expected = [string]$expectedReloadValues[$index]
        Assert-True (
            $actual -eq $expected
        ) "Reloaded field [$index] mismatch. Expected '$expected', got '$actual'."
    }

    Invoke-Button -Root $root -Name "File"
    Invoke-Button -Root $root -Name "Open Media File"
    Assert-ContainsName -Root $root -Expected "view: main-window"
    Assert-ContainsName -Root $root -Expected $openMediaFilePath

    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process = $null
    }

    $launch = Open-SmokeWindow -ResolvedBinaryPath $resolvedBinaryPath -ConfigPath $configPath -MediaSearchBrowsePath $mediaSearchDirectoryPath -OpenMediaFilePath $openMediaFilePath -EnableLoopbackSession $true
    $process = $launch.Process
    $root = $launch.Root
    Invoke-Button -Root $root -Name "Main Window"
    Assert-ContainsName -Root $root -Expected "view: main-window"
    $chatInput = Find-MainWindowChatInputEdit -Root $root
    Submit-EditValue -Root $root -Edit $chatInput -Value "helloloopback"
    Assert-ContainsName -Root $root -Expected "pending: send-chat-message"
    Invoke-Button -Root $root -Name "Complete"
    $chatInputAfterSend = Find-MainWindowChatInputEdit -Root $root
    Assert-True (
        [string]::IsNullOrEmpty((Get-EditValue -Edit $chatInputAfterSend))
    ) "Loopback session chat send should clear the chat draft after completion."

    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process = $null
    }

    $mockTcpServer = Start-MockSyncplayTcpServer -TempRootPath $tempRootPath -Username "smoke-user" -Room "smoke-room" -Name "mock-syncplay-server-primary" -SendPostChatState $true -PostChatPlaylistFiles @("postchat1.mkv", "postchat2.mkv") -PostChatReady $false -PostChatPaused $false -SendUserLeftOnSecondChat $true -UserLeftUsername "bob"
    $mockTcpReconnectServer = Start-MockSyncplayTcpServer -TempRootPath $tempRootPath -Username "smoke-user" -Room "smoke-room" -Name "mock-syncplay-server-reconnect" -PlaylistFiles @("reconnect1.mkv", "reconnect2.mkv") -Ready $false -Paused $false -SendPostChatState $true -PostChatPlaylistFiles @("reconnect-post1.mkv", "reconnect-post2.mkv") -PostChatReady $true -PostChatPaused $true -RemoteUsername "carol" -RemoteUserReady $false -RemoteUserController $false -RemoteUserFile "carol.mp4" -PostChatRemoteUsername "carol" -PostChatRemoteUserReady $true -PostChatRemoteUserController $true -PostChatRemoteUserFile "carol-post.mp4" -SendUserLeftOnSecondChat $true -UserLeftUsername "carol"
    $tcpPublicServersSpec =
        "[['Primary', '127.0.0.1:$($mockTcpServer.Port)'], ['Reconnect', '127.0.0.1:$($mockTcpReconnectServer.Port)']]"

    $launch = Open-SmokeWindow -ResolvedBinaryPath $resolvedBinaryPath -ConfigPath $configPath -MediaSearchBrowsePath $mediaSearchDirectoryPath -OpenMediaFilePath $openMediaFilePath -EnableTcpSession $true -TcpSessionPort $mockTcpServer.Port -PublicServersSpec $tcpPublicServersSpec
    $process = $launch.Process
    $root = $launch.Root
    Wait-ForMainWindowSessionState -Root $root -ExpectedPlaylistItem "episode2.mkv" -ExpectedUserRow "smoke-user: self=yes, ready=yes, controller=no" -ExpectedAdditionalNames @("bob: self=no, ready=yes, controller=yes") -ExpectedPausedValue "yes"
    $chatInput = Find-MainWindowChatInputEdit -Root $root
    Submit-EditValue -Root $root -Edit $chatInput -Value "hellotcp"
    Assert-ContainsName -Root $root -Expected "pending: send-chat-message"
    Invoke-Button -Root $root -Name "Complete"
    $chatInputAfterSend = Find-MainWindowChatInputEdit -Root $root
    Assert-True (
        [string]::IsNullOrEmpty((Get-EditValue -Edit $chatInputAfterSend))
    ) "TCP session chat send should clear the chat draft after completion."
    Wait-ForMainWindowSessionState -Root $root -ExpectedPlaylistItem "postchat2.mkv" -ExpectedUserRow "smoke-user: self=yes, ready=no, controller=no" -ExpectedAdditionalNames @("bob: self=no, ready=no, controller=no") -ExpectedPausedValue "no"
    $chatInput = Find-MainWindowChatInputEdit -Root $root
    Submit-EditValue -Root $root -Edit $chatInput -Value "goodbyeprimary"
    Assert-ContainsName -Root $root -Expected "pending: send-chat-message"
    Invoke-Button -Root $root -Name "Complete"
    Wait-ForMainWindowSessionState -Root $root -ExpectedPlaylistItem "postchat2.mkv" -ExpectedUserRow "smoke-user: self=yes, ready=no, controller=no" -ExpectedPausedValue "no" -UnexpectedNames @("bob: self=no, ready=no, controller=no")
    Start-Sleep -Milliseconds 500
    $mockTcpTraffic = Get-Content $mockTcpServer.LogPath -Raw
    Assert-True (
        $mockTcpTraffic.Contains('"Hello"')
    ) "Mock Syncplay TCP server did not receive the startup Hello."
    Assert-True (
        $mockTcpTraffic.Contains('"Chat"')
    ) "Mock Syncplay TCP server did not receive the outbound chat message."

    Invoke-Button -Root $root -Name "Public Servers"
    Assert-ContainsName -Root $root -Expected "view: public-servers"
    Invoke-Button -Root $root -Name "Reconnect: 127.0.0.1:$($mockTcpReconnectServer.Port)"
    Invoke-Button -Root $root -Name "Connect"
    Assert-ContainsName -Root $root -Expected "pending: connect-public-server"
    Invoke-Button -Root $root -Name "Complete"
    Start-Sleep -Milliseconds 500
    $mockReconnectTraffic = Get-Content $mockTcpReconnectServer.LogPath -Raw
    Assert-True (
        $mockReconnectTraffic.Contains('"Hello"')
    ) "Mock reconnect TCP server did not receive the replacement startup Hello."

    Wait-ForMainWindowSessionState -Root $root -ExpectedPlaylistItem "reconnect2.mkv" -ExpectedUserRow "smoke-user: self=yes, ready=no, controller=no" -ExpectedAdditionalNames @("carol: self=no, ready=no, controller=no") -ExpectedPausedValue "no"
    $chatInput = Find-MainWindowChatInputEdit -Root $root
    Submit-EditValue -Root $root -Edit $chatInput -Value "helloreconnect"
    Assert-ContainsName -Root $root -Expected "pending: send-chat-message"
    Invoke-Button -Root $root -Name "Complete"
    Wait-ForMainWindowSessionState -Root $root -ExpectedPlaylistItem "reconnect-post2.mkv" -ExpectedUserRow "smoke-user: self=yes, ready=yes, controller=no" -ExpectedAdditionalNames @("carol: self=no, ready=yes, controller=yes") -ExpectedPausedValue "yes"
    $chatInput = Find-MainWindowChatInputEdit -Root $root
    Submit-EditValue -Root $root -Edit $chatInput -Value "goodbyereconnect"
    Assert-ContainsName -Root $root -Expected "pending: send-chat-message"
    Invoke-Button -Root $root -Name "Complete"
    Wait-ForMainWindowSessionState -Root $root -ExpectedPlaylistItem "reconnect-post2.mkv" -ExpectedUserRow "smoke-user: self=yes, ready=yes, controller=no" -ExpectedPausedValue "yes" -UnexpectedNames @("carol: self=no, ready=yes, controller=yes")
    Start-Sleep -Milliseconds 500
    $mockReconnectTraffic = Get-Content $mockTcpReconnectServer.LogPath -Raw
    Assert-True (
        $mockReconnectTraffic.Contains('"Chat"')
    ) "Mock reconnect TCP server did not receive the post-reconnect outbound chat message."

    Write-Host "GUI smoke regression passed."
}
finally {
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
    Stop-MockSyncplayTcpServer -Server $mockTcpServer
    Stop-MockSyncplayTcpServer -Server $mockTcpReconnectServer
    Restore-SmokeEnvironment
}
