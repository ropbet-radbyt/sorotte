use std::{
    fs,
    io::{BufRead, BufReader, ErrorKind, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use syncplay_client_app::{
    legacy_settings::StoredClientSettingsMvp,
    legacy_syncplay_ini::{
        load_syncplay_ini_stored_client_settings_mvp_from_path,
        upsert_syncplay_ini_stored_client_settings_mvp_at_path,
    },
};
use syncplay_compat::LegacyServerPythonPeerHarness;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

struct NativeSmokeOptions {
    binary_path: Option<PathBuf>,
    timeout: Duration,
    format: OutputFormat,
    keep_open: bool,
}

struct NativeSmokeReport {
    binary_path: String,
    pid: u32,
    window_title: String,
    menu_labels: Vec<String>,
    menu_contract: String,
    accessible_name_count: usize,
    accessibility_contract: String,
    interaction_steps: Vec<String>,
    interaction_contract: String,
    duration_ms: u128,
    closed: bool,
}

#[derive(Clone, Copy)]
struct TcpSessionBootstrap<'a> {
    host: &'a str,
    port: u16,
    username: &'a str,
    room: &'a str,
}

#[derive(Clone, Copy)]
struct GuiLaunchConfig<'a> {
    config_path: &'a Path,
    media_search_browse_path: &'a Path,
    open_media_file_path: &'a Path,
    public_servers_spec: &'a str,
    tcp_session: Option<TcpSessionBootstrap<'a>>,
    loopback_session: Option<(&'a str, &'a str)>,
}

struct MockSessionServer {
    address: String,
    port: u16,
    hello_rx: mpsc::Receiver<String>,
    chat_rx: mpsc::Receiver<String>,
    release_tx: mpsc::Sender<()>,
    join_handle: Option<thread::JoinHandle<Result<(), String>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NativeControlKind {
    Any,
    Button,
    MenuItem,
}

impl NativeControlKind {
    fn label(self) -> &'static str {
        match self {
            Self::Any => "control",
            Self::Button => "button",
            Self::MenuItem => "menu-item",
        }
    }

    #[cfg(target_os = "windows")]
    fn matches_control_type(
        self,
        control_type: windows::Win32::UI::Accessibility::UIA_CONTROLTYPE_ID,
    ) -> bool {
        use windows::Win32::UI::Accessibility::{
            UIA_ButtonControlTypeId, UIA_MenuItemControlTypeId,
        };

        match self {
            Self::Any => true,
            Self::Button => control_type == UIA_ButtonControlTypeId,
            Self::MenuItem => control_type == UIA_MenuItemControlTypeId,
        }
    }
}

const DEFAULT_PUBLIC_SERVERS_SPEC: &str =
    "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]";
const DLL_INIT_FAILED_STATUS: u32 = 0xC000_0142;
const LAUNCH_ATTEMPTS: usize = 2;
const TRANSPORT_SESSION_USERNAME: &str = "smoke-user";
const TRANSPORT_SESSION_ROOM: &str = "smoke-room";
const LIVE_PYTHON_INTEROP_LOCAL_USERNAME: &str = "interop-gui-user";
const LIVE_PYTHON_INTEROP_PEER_USERNAME: &str = "interop-py-peer";
const LIVE_PYTHON_INTEROP_ROOM: &str = "interop-room";
const LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE: &str = "hello from gui";
const LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE: &str = "hello from python";
const LIVE_PYTHON_INTEROP_LOCAL_ROW_NAME: &str =
    "interop-gui-user: self=yes, ready=no, controller=no";
const LIVE_PYTHON_INTEROP_LOCAL_READY_ROW_NAME: &str =
    "interop-gui-user: self=yes, ready=yes, controller=no";
const LIVE_PYTHON_INTEROP_PEER_ROW_NAME: &str = "interop-py-peer: self=no, ready=no, controller=no";
const LIVE_PYTHON_INTEROP_PEER_READY_ROW_NAME: &str =
    "interop-py-peer: self=no, ready=yes, controller=no";
const CONFIG_HOST_VALUE: &str = "syncplay.example";
const CONFIG_PORT_VALUE: &str = "8999";
const CONFIG_USERNAME_VALUE: &str = "smoke-user";
const CONFIG_ROOM_VALUE: &str = "smoke-room";
const CONFIG_PLAYER_PATH_VALUE: &str = "C:\\Windows\\System32\\notepad.exe";
const TRUSTED_DOMAINS_EDIT_INDEX: usize = 6;
const TRUSTED_DOMAINS_VALUE: &str = "youtube.com; *.example.com/videos";
const CUSTOM_SERVER_LABEL: &str = "Custom";
const CUSTOM_SERVER_HOST: &str = "custom.example";
const CUSTOM_SERVER_PORT: &str = "9001";
const CUSTOM_SERVER_ADDRESS: &str = "custom.example:9001";
const CUSTOM_SERVER_ROW_NAME: &str = "Custom: custom.example:9001";
const MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS: f64 = 3.0;
const MEDIA_SEARCH_TIMEOUT_SECONDS: f64 = 30.0;
const MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS: f64 = 2.5;
const MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS: f64 = 7.5;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_WIDTH: i32 = 1700;
#[cfg(target_os = "windows")]
const SMOKE_WINDOW_HEIGHT: i32 = 1100;
const CONNECT_NO_SESSION_ERROR: &str = "error: Public server connect requires a session runtime connection; the selected server was not contacted.";
const REFRESH_NO_SESSION_ERROR: &str = "error: Public server refresh requires a session runtime connection; the server list was not refreshed.";
const SEARCH_NO_SESSION_ERROR: &str =
    "error: Missing-media search requires a session runtime connection; no search was performed.";

trait NativeGuiDriver {
    type WindowHandle: Copy;

    fn find_main_window(&self, pid: u32) -> Result<Option<Self::WindowHandle>, String>;
    fn prepare_window_for_smoke(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn scroll_active_view_page_down(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn scroll_active_view_page_up(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn window_title(&self, window: Self::WindowHandle) -> Result<String, String>;
    fn accessible_names(&self, window: Self::WindowHandle) -> Result<Vec<String>, String>;
    fn top_level_menu_labels(&self, window: Self::WindowHandle) -> Result<Vec<String>, String>;
    fn count_named_controls(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<usize, String>;
    fn editable_text_input_count(&self, window: Self::WindowHandle) -> Result<usize, String>;
    fn get_edit_value_by_index(
        &self,
        window: Self::WindowHandle,
        edit_index: usize,
    ) -> Result<String, String>;
    fn get_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
    ) -> Result<String, String>;
    fn set_edit_value_by_index(
        &self,
        window: Self::WindowHandle,
        edit_index: usize,
        value: &str,
    ) -> Result<(), String>;
    fn set_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
        value: &str,
        submit: bool,
    ) -> Result<(), String>;
    fn invoke_named_control(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String>;
    fn close_window(&self, window: Self::WindowHandle) -> Result<(), String>;
}

#[cfg(target_os = "windows")]
type PlatformWindowHandle = windows_sys::Win32::Foundation::HWND;

#[cfg(not(target_os = "windows"))]
type PlatformWindowHandle = ();

#[cfg(target_os = "windows")]
#[derive(Default)]
struct PlatformNativeGuiDriver;

#[cfg(not(target_os = "windows"))]
#[derive(Default)]
struct PlatformNativeGuiDriver;

#[cfg(target_os = "windows")]
impl PlatformNativeGuiDriver {
    fn window_text_for_handle(window: PlatformWindowHandle) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

        let text_length = unsafe { GetWindowTextLengthW(window) };
        if text_length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; text_length as usize + 1];
        let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
        if copied <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..copied as usize])
    }

    fn with_ui_automation<T, F>(
        window: PlatformWindowHandle,
        operation_label: &str,
        run: F,
    ) -> Result<T, String>
    where
        F: FnOnce(
            &windows::Win32::UI::Accessibility::IUIAutomation,
            &windows::Win32::UI::Accessibility::IUIAutomationElement,
        ) -> Result<T, String>,
    {
        use windows::{
            Win32::{
                Foundation::HWND,
                System::Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoUninitialize,
                },
                UI::Accessibility::{CUIAutomation, IUIAutomation},
            },
            core::HRESULT,
        };

        struct ComScope {
            should_uninitialize: bool,
        }

        impl ComScope {
            fn initialize(operation_label: &str) -> Result<Self, String> {
                let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                if hr.is_ok() {
                    return Ok(Self {
                        should_uninitialize: true,
                    });
                }
                let rpc_e_changed_mode = HRESULT(0x8001_0106u32 as i32);
                if hr == rpc_e_changed_mode {
                    return Ok(Self {
                        should_uninitialize: false,
                    });
                }
                Err(format!(
                    "failed to initialize COM for {operation_label}: 0x{:08X}",
                    hr.0 as u32
                ))
            }
        }

        impl Drop for ComScope {
            fn drop(&mut self) {
                if self.should_uninitialize {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }

        let _com = ComScope::initialize(operation_label)?;
        let automation: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("failed to create IUIAutomation instance: {error}"))?
        };
        let root = unsafe {
            automation
                .ElementFromHandle(HWND(window))
                .map_err(|error| {
                    format!("failed to map main window handle into UI Automation element: {error}")
                })?
        };
        run(&automation, &root)
    }

    fn collect_subtree_elements(
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<windows::Win32::UI::Accessibility::IUIAutomationElementArray, String> {
        use windows::Win32::UI::Accessibility::TreeScope_Subtree;

        let all_condition = unsafe {
            automation.CreateTrueCondition().map_err(|error| {
                format!("failed to create UI Automation true condition: {error}")
            })?
        };
        unsafe {
            root.FindAll(TreeScope_Subtree, &all_condition)
                .map_err(|error| {
                    format!("failed to enumerate UI Automation element subtree: {error}")
                })
        }
    }

    fn collect_accessible_names(window: PlatformWindowHandle) -> Result<Vec<String>, String> {
        Self::with_ui_automation(window, "accessibility discovery", |automation, root| {
            let elements = Self::collect_subtree_elements(automation, root)?;
            let length = unsafe {
                elements.Length().map_err(|error| {
                    format!("failed to read UI Automation element count: {error}")
                })?
            };
            let mut names = Vec::new();
            for index in 0..length {
                let element = unsafe {
                    elements.GetElement(index).map_err(|error| {
                        format!("failed to fetch UI Automation element at index {index}: {error}")
                    })?
                };
                let name = unsafe {
                    element.CurrentName().map_err(|error| {
                        format!("failed to read UI Automation element name: {error}")
                    })?
                };
                let trimmed = name.to_string().trim().to_owned();
                if !trimmed.is_empty() {
                    names.push(trimmed);
                }
            }
            names.sort();
            names.dedup();
            Ok(names)
        })
    }

    fn send_keyboard_inputs(
        inputs: &mut [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
    ) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>()
                    as i32,
            )
        };
        if sent != inputs.len() as u32 {
            return Err(format!(
                "SendInput sent {sent} keyboard events out of {}",
                inputs.len()
            ));
        }
        Ok(())
    }

    fn keyboard_input_for_vk(
        vk: u16,
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
        };

        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn keyboard_input_for_unicode(
        ch: u16,
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_UNICODE,
        };

        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: ch,
                    dwFlags: flags | KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send_select_all_backspace_and_type(value: &str) -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            KEYEVENTF_KEYUP, VK_BACK, VK_CONTROL,
        };

        let mut controls = [
            Self::keyboard_input_for_vk(VK_CONTROL, 0),
            Self::keyboard_input_for_vk('A' as u16, 0),
            Self::keyboard_input_for_vk('A' as u16, KEYEVENTF_KEYUP),
            Self::keyboard_input_for_vk(VK_CONTROL, KEYEVENTF_KEYUP),
            Self::keyboard_input_for_vk(VK_BACK, 0),
            Self::keyboard_input_for_vk(VK_BACK, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut controls)?;

        if value.is_empty() {
            return Ok(());
        }

        let mut text_inputs = Vec::with_capacity(value.encode_utf16().count() * 2);
        for ch in value.encode_utf16() {
            text_inputs.push(Self::keyboard_input_for_unicode(ch, 0));
            text_inputs.push(Self::keyboard_input_for_unicode(ch, KEYEVENTF_KEYUP));
        }
        Self::send_keyboard_inputs(&mut text_inputs)
    }

    fn send_enter_key() -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_RETURN};

        let mut enter_inputs = [
            Self::keyboard_input_for_vk(VK_RETURN, 0),
            Self::keyboard_input_for_vk(VK_RETURN, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut enter_inputs)
    }

    fn send_page_down_key() -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_NEXT};

        let mut page_down_inputs = [
            Self::keyboard_input_for_vk(VK_NEXT, 0),
            Self::keyboard_input_for_vk(VK_NEXT, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut page_down_inputs)
    }

    fn send_page_up_key() -> Result<(), String> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_PRIOR};

        let mut page_up_inputs = [
            Self::keyboard_input_for_vk(VK_PRIOR, 0),
            Self::keyboard_input_for_vk(VK_PRIOR, KEYEVENTF_KEYUP),
        ];
        Self::send_keyboard_inputs(&mut page_up_inputs)
    }

    fn collect_editable_elements(
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<Vec<windows::Win32::UI::Accessibility::IUIAutomationElement>, String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationElement, IUIAutomationValuePattern, UIA_EditControlTypeId,
            UIA_ValuePatternId,
        };

        let elements = Self::collect_subtree_elements(automation, root)?;
        let length = unsafe {
            elements
                .Length()
                .map_err(|error| format!("failed to read UI Automation element count: {error}"))?
        };

        let mut edit_elements: Vec<IUIAutomationElement> = Vec::new();
        for index in 0..length {
            let element = unsafe {
                match elements.GetElement(index) {
                    Ok(element) => element,
                    Err(_) => continue,
                }
            };
            let current_control_type = unsafe {
                match element.CurrentControlType() {
                    Ok(control_type) => control_type,
                    Err(_) => continue,
                }
            };
            if current_control_type != UIA_EditControlTypeId {
                continue;
            }
            let is_enabled = unsafe {
                match element.CurrentIsEnabled() {
                    Ok(enabled) => enabled.as_bool(),
                    Err(_) => false,
                }
            };
            if !is_enabled {
                continue;
            }
            let is_offscreen = unsafe {
                match element.CurrentIsOffscreen() {
                    Ok(offscreen) => offscreen.as_bool(),
                    Err(_) => false,
                }
            };
            if is_offscreen {
                continue;
            }

            let read_only = unsafe {
                match element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                    Ok(pattern) => pattern
                        .CurrentIsReadOnly()
                        .map(|flag| flag.as_bool())
                        .unwrap_or(true),
                    Err(_) => false,
                }
            };
            if read_only {
                continue;
            }
            edit_elements.push(element);
        }
        Ok(edit_elements)
    }

    fn set_edit_element_value(
        window: PlatformWindowHandle,
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        value: &str,
        submit: bool,
    ) -> Result<(), String> {
        use windows::{Win32::UI::Accessibility::IUIAutomationValuePattern, core::BSTR};
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        let value_pattern = unsafe {
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(
                windows::Win32::UI::Accessibility::UIA_ValuePatternId,
            )
        }
        .ok();
        if let Some(pattern) = value_pattern.as_ref() {
            let value_bstr = BSTR::from(value);
            if unsafe { pattern.SetValue(&value_bstr) }.is_ok() {
                thread::sleep(Duration::from_millis(120));
                let actual = unsafe { pattern.CurrentValue() }
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                if actual == value {
                    if !submit {
                        return Ok(());
                    }
                    unsafe {
                        SetForegroundWindow(window);
                        element.SetFocus().map_err(|error| {
                            format!("failed to focus edit field for submit key entry: {error}")
                        })?;
                    }
                    thread::sleep(Duration::from_millis(120));
                    Self::send_enter_key().map_err(|error| {
                        format!("failed to submit edit entry with Enter key: {error}")
                    })?;
                    thread::sleep(Duration::from_millis(120));
                    return Ok(());
                }
            }
        }

        unsafe {
            SetForegroundWindow(window);
            element.SetFocus().map_err(|error| {
                format!("failed to focus edit field for keyboard entry: {error}")
            })?;
        }
        thread::sleep(Duration::from_millis(120));
        Self::send_select_all_backspace_and_type(value)
            .map_err(|error| format!("failed keyboard fallback text entry: {error}"))?;
        thread::sleep(Duration::from_millis(120));
        if submit {
            Self::send_enter_key()
                .map_err(|error| format!("failed to submit edit entry with Enter key: {error}"))?;
            thread::sleep(Duration::from_millis(120));
            return Ok(());
        }

        let Some(verification_pattern) = value_pattern else {
            return Ok(());
        };
        let actual = unsafe { verification_pattern.CurrentValue() }
            .map(|value| value.to_string())
            .unwrap_or_default();
        if actual == value {
            Ok(())
        } else {
            Err(format!(
                "keyboard fallback set edit field to {actual:?}, expected {value:?}"
            ))
        }
    }

    fn invoke_named_control(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
            IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern,
            UIA_ExpandCollapsePatternId, UIA_InvokePatternId, UIA_SelectionItemPatternId,
            UIA_TogglePatternId,
        };

        Self::with_ui_automation(window, "UI Automation interaction", |automation, root| {
            let elements = Self::collect_subtree_elements(automation, root)?;
            let length = unsafe {
                elements.Length().map_err(|error| {
                    format!("failed to read UI Automation element count: {error}")
                })?
            };

            let mut candidates = Vec::new();
            let mut matching_states = Vec::new();
            for index in 0..length {
                let element = unsafe {
                    match elements.GetElement(index) {
                        Ok(element) => element,
                        Err(_) => continue,
                    }
                };
                let current_name = unsafe {
                    match element.CurrentName() {
                        Ok(name_value) => name_value.to_string().trim().to_owned(),
                        Err(_) => continue,
                    }
                };
                if current_name != name {
                    continue;
                }

                let current_control_type = unsafe {
                    match element.CurrentControlType() {
                        Ok(control_type) => control_type,
                        Err(_) => continue,
                    }
                };
                if !control_kind.matches_control_type(current_control_type) {
                    continue;
                }

                let is_enabled = unsafe {
                    match element.CurrentIsEnabled() {
                        Ok(enabled) => enabled.as_bool(),
                        Err(_) => false,
                    }
                };
                let is_offscreen = unsafe {
                    match element.CurrentIsOffscreen() {
                        Ok(offscreen) => offscreen.as_bool(),
                        Err(_) => false,
                    }
                };
                matching_states.push(format!(
                    "enabled={}, offscreen={}",
                    bool_label(is_enabled),
                    bool_label(is_offscreen)
                ));
                if !is_enabled {
                    continue;
                }
                if is_offscreen {
                    continue;
                }

                candidates.push(element);
            }

            if candidates.is_empty() {
                let matching_state_summary = if matching_states.is_empty() {
                    "none".to_owned()
                } else {
                    matching_states.join(", ")
                };
                return Err(format!(
                    "did not find an enabled {} named {name:?} in the accessibility tree; matching states: {}",
                    control_kind.label(),
                    matching_state_summary,
                ));
            }

            let mut invoke_errors = Vec::new();
            for candidate in candidates {
                let mut candidate_errors = Vec::new();

                let invoke_result = (|| -> Result<(), String> {
                    let invoke_pattern: IUIAutomationInvokePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_InvokePatternId)
                            .map_err(|error| format!("invoke pattern unavailable: {error}"))?
                    };
                    unsafe { invoke_pattern.Invoke() }
                        .map_err(|error| format!("invoke pattern action failed: {error}"))
                })();
                if invoke_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(invoke_result.err().unwrap_or_default());

                let selection_result = (|| -> Result<(), String> {
                    let selection_pattern: IUIAutomationSelectionItemPattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_SelectionItemPatternId)
                            .map_err(|error| {
                                format!("selection-item pattern unavailable: {error}")
                            })?
                    };
                    unsafe { selection_pattern.Select() }
                        .map_err(|error| format!("selection-item action failed: {error}"))
                })();
                if selection_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(selection_result.err().unwrap_or_default());

                let toggle_result = (|| -> Result<(), String> {
                    let toggle_pattern: IUIAutomationTogglePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_TogglePatternId)
                            .map_err(|error| format!("toggle pattern unavailable: {error}"))?
                    };
                    unsafe { toggle_pattern.Toggle() }
                        .map_err(|error| format!("toggle action failed: {error}"))
                })();
                if toggle_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(toggle_result.err().unwrap_or_default());

                let expand_result = (|| -> Result<(), String> {
                    let expand_pattern: IUIAutomationExpandCollapsePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
                            .map_err(|error| {
                                format!("expand-collapse pattern unavailable: {error}")
                            })?
                    };
                    unsafe { expand_pattern.Expand() }
                        .map_err(|error| format!("expand action failed: {error}"))
                })();
                if expand_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(expand_result.err().unwrap_or_default());

                invoke_errors.push(candidate_errors.join("; "));
            }

            Err(format!(
                "failed to invoke {} named {name:?}: {}",
                control_kind.label(),
                invoke_errors.join("; ")
            ))
        })
    }

    fn count_named_controls(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<usize, String> {
        Self::with_ui_automation(
            window,
            "UI Automation control counting",
            |automation, root| {
                let elements = Self::collect_subtree_elements(automation, root)?;
                let length = unsafe {
                    elements.Length().map_err(|error| {
                        format!("failed to read UI Automation element count: {error}")
                    })?
                };

                let mut count = 0usize;
                for index in 0..length {
                    let element = unsafe {
                        match elements.GetElement(index) {
                            Ok(element) => element,
                            Err(_) => continue,
                        }
                    };
                    let current_name = unsafe {
                        match element.CurrentName() {
                            Ok(name_value) => name_value.to_string().trim().to_owned(),
                            Err(_) => continue,
                        }
                    };
                    if current_name != name {
                        continue;
                    }

                    let current_control_type = unsafe {
                        match element.CurrentControlType() {
                            Ok(control_type) => control_type,
                            Err(_) => continue,
                        }
                    };
                    if !control_kind.matches_control_type(current_control_type) {
                        continue;
                    }

                    let is_enabled = unsafe {
                        match element.CurrentIsEnabled() {
                            Ok(enabled) => enabled.as_bool(),
                            Err(_) => false,
                        }
                    };
                    if !is_enabled {
                        continue;
                    }
                    let is_offscreen = unsafe {
                        match element.CurrentIsOffscreen() {
                            Ok(offscreen) => offscreen.as_bool(),
                            Err(_) => false,
                        }
                    };
                    if is_offscreen {
                        continue;
                    }
                    count += 1;
                }
                Ok(count)
            },
        )
    }

    fn editable_text_input_count(window: PlatformWindowHandle) -> Result<usize, String> {
        use windows::Win32::UI::Accessibility::UIA_EditControlTypeId;

        Self::with_ui_automation(
            window,
            "UI Automation editable text count",
            |automation, root| {
                let elements = Self::collect_subtree_elements(automation, root)?;
                let length = unsafe {
                    elements.Length().map_err(|error| {
                        format!("failed to read UI Automation element count: {error}")
                    })?
                };

                let mut count = 0usize;
                for index in 0..length {
                    let element = unsafe {
                        match elements.GetElement(index) {
                            Ok(element) => element,
                            Err(_) => continue,
                        }
                    };
                    let current_control_type = unsafe {
                        match element.CurrentControlType() {
                            Ok(control_type) => control_type,
                            Err(_) => continue,
                        }
                    };
                    if current_control_type != UIA_EditControlTypeId {
                        continue;
                    }
                    let is_enabled = unsafe {
                        match element.CurrentIsEnabled() {
                            Ok(enabled) => enabled.as_bool(),
                            Err(_) => false,
                        }
                    };
                    if is_enabled {
                        let is_offscreen = unsafe {
                            match element.CurrentIsOffscreen() {
                                Ok(offscreen) => offscreen.as_bool(),
                                Err(_) => false,
                            }
                        };
                        if is_offscreen {
                            continue;
                        }
                        count += 1;
                    }
                }
                Ok(count)
            },
        )
    }

    fn read_edit_element_value(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<String, String> {
        use windows::Win32::UI::Accessibility::{IUIAutomationValuePattern, UIA_ValuePatternId};

        let pattern: IUIAutomationValuePattern = unsafe {
            element
                .GetCurrentPatternAs(UIA_ValuePatternId)
                .map_err(|error| format!("value pattern unavailable for edit control: {error}"))?
        };
        let value = unsafe { pattern.CurrentValue() }
            .map_err(|error| format!("failed to read edit control value: {error}"))?;
        Ok(value.to_string())
    }

    fn get_edit_value_by_index(
        window: PlatformWindowHandle,
        edit_index: usize,
    ) -> Result<String, String> {
        Self::with_ui_automation(
            window,
            "UI Automation edit value lookup",
            |automation, root| {
                let mut edit_elements = Self::collect_editable_elements(automation, root)?;
                if edit_elements.len() <= edit_index {
                    return Err(format!(
                        "edit field index {edit_index} was requested, but only {} editable text fields were found",
                        edit_elements.len()
                    ));
                }
                let element = edit_elements.remove(edit_index);
                Self::read_edit_element_value(&element).map_err(|error| {
                    format!("failed to read edit field index {edit_index}: {error}")
                })
            },
        )
    }

    fn get_named_edit_value(window: PlatformWindowHandle, name: &str) -> Result<String, String> {
        Self::with_ui_automation(
            window,
            "UI Automation named edit value lookup",
            |automation, root| {
                let edit_elements = Self::collect_editable_elements(automation, root)?;
                let mut fallback_last_edit = None;
                let mut available_names = Vec::new();
                for element in edit_elements {
                    fallback_last_edit = Some(element.clone());
                    let element_name = unsafe { element.CurrentName() }
                        .map(|value| value.to_string().trim().to_owned())
                        .unwrap_or_default();
                    if !element_name.is_empty() {
                        available_names.push(element_name.clone());
                    }
                    if element_name == name {
                        return Self::read_edit_element_value(&element).map_err(|error| {
                            format!("failed to read edit field named {name:?}: {error}")
                        });
                    }
                }
                if let Some(element) = fallback_last_edit {
                    return Self::read_edit_element_value(&element).map_err(|error| {
                    format!(
                        "failed to read fallback unnamed edit field while targeting {name:?}: {error}"
                    )
                });
                }
                available_names.sort();
                available_names.dedup();
                if available_names.is_empty() {
                    Err(format!(
                        "edit field named {name:?} was not found; no editable controls were discovered"
                    ))
                } else {
                    Err(format!(
                        "edit field named {name:?} was not found; available editable names: {}",
                        available_names.join(", ")
                    ))
                }
            },
        )
    }
    fn set_edit_value_by_index(
        window: PlatformWindowHandle,
        edit_index: usize,
        value: &str,
    ) -> Result<(), String> {
        Self::with_ui_automation(window, "UI Automation edit entry", |automation, root| {
            let mut edit_elements = Self::collect_editable_elements(automation, root)?;

            if edit_elements.len() <= edit_index {
                return Err(format!(
                    "edit field index {edit_index} was requested, but only {} editable text fields were found",
                    edit_elements.len()
                ));
            }
            let element = edit_elements.remove(edit_index);
            Self::set_edit_element_value(window, &element, value, false)
                .map_err(|error| format!("failed to write edit field index {edit_index}: {error}"))
        })
    }

    fn set_named_edit_value(
        window: PlatformWindowHandle,
        name: &str,
        value: &str,
        submit: bool,
    ) -> Result<(), String> {
        Self::with_ui_automation(
            window,
            "UI Automation named edit entry",
            |automation, root| {
                let edit_elements = Self::collect_editable_elements(automation, root)?;
                let mut fallback_last_edit = None;
                let mut available_names = Vec::new();
                for element in edit_elements {
                    fallback_last_edit = Some(element.clone());
                    let element_name = unsafe { element.CurrentName() }
                        .map(|value| value.to_string().trim().to_owned())
                        .unwrap_or_default();
                    if !element_name.is_empty() {
                        available_names.push(element_name.clone());
                    }
                    if element_name == name {
                        return Self::set_edit_element_value(window, &element, value, submit)
                            .map_err(|error| {
                                format!("failed to write edit field named {name:?}: {error}")
                            });
                    }
                }
                if let Some(element) = fallback_last_edit {
                    return Self::set_edit_element_value(window, &element, value, submit).map_err(
                        |error| {
                            format!(
                                "failed to write fallback unnamed edit field while targeting {name:?}: {error}"
                            )
                        },
                    );
                }
                available_names.sort();
                available_names.dedup();
                if available_names.is_empty() {
                    Err(format!(
                        "edit field named {name:?} was not found; no editable controls were discovered"
                    ))
                } else {
                    Err(format!(
                        "edit field named {name:?} was not found; available editable names: {}",
                        available_names.join(", ")
                    ))
                }
            },
        )
    }
}

#[cfg(target_os = "windows")]
struct FindWindowContext {
    pid: u32,
    window: PlatformWindowHandle,
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_windows_for_process(
    window: PlatformWindowHandle,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::core::BOOL {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};

    let context = unsafe { &mut *(lparam as *mut FindWindowContext) };
    let mut window_pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(window, &mut window_pid);
    }
    if window_pid != context.pid {
        return 1;
    }
    if unsafe { IsWindowVisible(window) } == 0 {
        return 1;
    }

    let title = PlatformNativeGuiDriver::window_text_for_handle(window);
    if title.trim().is_empty() {
        return 1;
    }

    context.window = window;
    0
}

#[cfg(target_os = "windows")]
impl NativeGuiDriver for PlatformNativeGuiDriver {
    type WindowHandle = PlatformWindowHandle;

    fn find_main_window(&self, pid: u32) -> Result<Option<Self::WindowHandle>, String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;

        let mut context = FindWindowContext {
            pid,
            window: std::ptr::null_mut(),
        };
        unsafe {
            EnumWindows(
                Some(enum_windows_for_process),
                (&mut context as *mut FindWindowContext) as isize,
            );
        }
        if context.window.is_null() {
            Ok(None)
        } else {
            Ok(Some(context.window))
        }
    }

    fn prepare_window_for_smoke(&self, window: Self::WindowHandle) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
        };

        unsafe {
            SetForegroundWindow(window);
            let result = SetWindowPos(
                window,
                std::ptr::null_mut(),
                0,
                0,
                SMOKE_WINDOW_WIDTH,
                SMOKE_WINDOW_HEIGHT,
                SWP_NOMOVE | SWP_NOZORDER,
            );
            if result == 0 {
                return Err("failed to set native smoke window size".to_owned());
            }
        }
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn scroll_active_view_page_down(&self, window: Self::WindowHandle) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        unsafe {
            SetForegroundWindow(window);
        }
        thread::sleep(Duration::from_millis(80));
        Self::send_page_down_key()
            .map_err(|error| format!("failed to send PageDown to native smoke window: {error}"))?;
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn scroll_active_view_page_up(&self, window: Self::WindowHandle) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        unsafe {
            SetForegroundWindow(window);
        }
        thread::sleep(Duration::from_millis(80));
        Self::send_page_up_key()
            .map_err(|error| format!("failed to send PageUp to native smoke window: {error}"))?;
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn window_title(&self, window: Self::WindowHandle) -> Result<String, String> {
        Ok(Self::window_text_for_handle(window))
    }

    fn accessible_names(&self, window: Self::WindowHandle) -> Result<Vec<String>, String> {
        Self::collect_accessible_names(window)
    }

    fn top_level_menu_labels(&self, window: Self::WindowHandle) -> Result<Vec<String>, String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetMenu, GetMenuItemCount, GetMenuStringW, MF_BYPOSITION,
        };

        let menu = unsafe { GetMenu(window) };
        if menu.is_null() {
            return Ok(Vec::new());
        }
        let count = unsafe { GetMenuItemCount(menu) };
        if count < 0 {
            return Err("could not inspect top-level menu item count".to_owned());
        }

        let mut labels = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut buffer = vec![0u16; 256];
            let copied = unsafe {
                GetMenuStringW(
                    menu,
                    index as u32,
                    buffer.as_mut_ptr(),
                    buffer.len() as i32,
                    MF_BYPOSITION,
                )
            };
            if copied <= 0 {
                continue;
            }
            labels.push(String::from_utf16_lossy(&buffer[..copied as usize]));
        }
        Ok(labels)
    }

    fn count_named_controls(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<usize, String> {
        Self::count_named_controls(window, name, control_kind)
    }

    fn editable_text_input_count(&self, window: Self::WindowHandle) -> Result<usize, String> {
        Self::editable_text_input_count(window)
    }

    fn get_edit_value_by_index(
        &self,
        window: Self::WindowHandle,
        edit_index: usize,
    ) -> Result<String, String> {
        Self::get_edit_value_by_index(window, edit_index)
    }

    fn get_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
    ) -> Result<String, String> {
        Self::get_named_edit_value(window, name)
    }

    fn set_edit_value_by_index(
        &self,
        window: Self::WindowHandle,
        edit_index: usize,
        value: &str,
    ) -> Result<(), String> {
        Self::set_edit_value_by_index(window, edit_index, value)
    }

    fn set_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
        value: &str,
        submit: bool,
    ) -> Result<(), String> {
        Self::set_named_edit_value(window, name, value, submit)
    }

    fn invoke_named_control(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        Self::invoke_named_control(window, name, control_kind)
    }

    fn close_window(&self, window: Self::WindowHandle) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CLOSE};

        unsafe {
            SendMessageW(window, WM_CLOSE, 0, 0);
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
impl NativeGuiDriver for PlatformNativeGuiDriver {
    type WindowHandle = PlatformWindowHandle;

    fn find_main_window(&self, _pid: u32) -> Result<Option<Self::WindowHandle>, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn prepare_window_for_smoke(&self, _window: Self::WindowHandle) -> Result<(), String> {
        Ok(())
    }

    fn scroll_active_view_page_down(&self, _window: Self::WindowHandle) -> Result<(), String> {
        Ok(())
    }

    fn scroll_active_view_page_up(&self, _window: Self::WindowHandle) -> Result<(), String> {
        Ok(())
    }

    fn window_title(&self, _window: Self::WindowHandle) -> Result<String, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn accessible_names(&self, _window: Self::WindowHandle) -> Result<Vec<String>, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn top_level_menu_labels(&self, _window: Self::WindowHandle) -> Result<Vec<String>, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn count_named_controls(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
        _control_kind: NativeControlKind,
    ) -> Result<usize, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn editable_text_input_count(&self, _window: Self::WindowHandle) -> Result<usize, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn get_edit_value_by_index(
        &self,
        _window: Self::WindowHandle,
        _edit_index: usize,
    ) -> Result<String, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn get_named_edit_value(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
    ) -> Result<String, String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn set_edit_value_by_index(
        &self,
        _window: Self::WindowHandle,
        _edit_index: usize,
        _value: &str,
    ) -> Result<(), String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn set_named_edit_value(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
        _value: &str,
        _submit: bool,
    ) -> Result<(), String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn invoke_named_control(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
        _control_kind: NativeControlKind,
    ) -> Result<(), String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }

    fn close_window(&self, _window: Self::WindowHandle) -> Result<(), String> {
        Err("native smoke is currently implemented only on Windows".to_owned())
    }
}

impl NativeSmokeReport {
    fn render_text(&self) -> String {
        format!(
            "result=ok\nbinary={}\npid={}\nwindow_title={}\nmenu_labels={}\nmenu_contract={}\naccessible_name_count={}\naccessibility_contract={}\ninteraction_steps={}\ninteraction_contract={}\nclosed={}\nduration_ms={}\n",
            self.binary_path,
            self.pid,
            self.window_title,
            self.menu_labels.join("|"),
            self.menu_contract,
            self.accessible_name_count,
            self.accessibility_contract,
            self.interaction_steps.join("|"),
            self.interaction_contract,
            self.closed,
            self.duration_ms
        )
    }

    fn render_json(&self) -> String {
        let labels = self
            .menu_labels
            .iter()
            .map(|label| render_json_string(label))
            .collect::<Vec<_>>()
            .join(",");
        let interaction_steps = self
            .interaction_steps
            .iter()
            .map(|step| render_json_string(step))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"result\":\"ok\",\"binary\":{},\"pid\":{},\"window_title\":{},\"menu_labels\":[{}],\"menu_contract\":{},\"accessible_name_count\":{},\"accessibility_contract\":{},\"interaction_steps\":[{}],\"interaction_contract\":{},\"closed\":{},\"duration_ms\":{}}}\n",
            render_json_string(&self.binary_path),
            self.pid,
            render_json_string(&self.window_title),
            labels,
            render_json_string(&self.menu_contract),
            self.accessible_name_count,
            render_json_string(&self.accessibility_contract),
            interaction_steps,
            render_json_string(&self.interaction_contract),
            if self.closed { "true" } else { "false" },
            self.duration_ms
        )
    }

    fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Text => self.render_text(),
            OutputFormat::Json => self.render_json(),
        }
    }
}

fn render_json_string(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => rendered.push_str("\\\\"),
            '"' => rendered.push_str("\\\""),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            _ => rendered.push(ch),
        }
    }
    rendered.push('"');
    rendered
}

fn render_error(error: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format!("result=error\nerror={error}\n"),
        OutputFormat::Json => {
            format!(
                "{{\"result\":\"error\",\"error\":{}}}\n",
                render_json_string(error)
            )
        }
    }
}

fn parse_timeout_ms(token: &str) -> Result<Duration, String> {
    let timeout_ms = token
        .parse::<u64>()
        .map_err(|_| format!("--timeout-ms requires a positive integer, got {token:?}"))?;
    if timeout_ms == 0 {
        return Err("--timeout-ms must be greater than zero".to_owned());
    }
    Ok(Duration::from_millis(timeout_ms))
}

fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn parse_options(args: &[String]) -> Result<NativeSmokeOptions, String> {
    let mut options = NativeSmokeOptions {
        binary_path: None,
        timeout: Duration::from_millis(10_000),
        format: OutputFormat::Text,
        keep_open: false,
    };

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--binary" => {
                if index + 1 >= args.len() {
                    return Err("--binary requires a path".to_owned());
                }
                options.binary_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--timeout-ms" => {
                if index + 1 >= args.len() {
                    return Err("--timeout-ms requires an integer value".to_owned());
                }
                options.timeout = parse_timeout_ms(&args[index + 1])?;
                index += 2;
            }
            "--json" => {
                options.format = OutputFormat::Json;
                index += 1;
            }
            "--text" => {
                options.format = OutputFormat::Text;
                index += 1;
            }
            "--keep-open" => {
                options.keep_open = true;
                index += 1;
            }
            "--help" | "-h" => {
                return Err(native_smoke_usage().to_owned());
            }
            argument => {
                return Err(format!("unknown argument {argument:?}"));
            }
        }
    }

    Ok(options)
}

fn native_smoke_usage() -> &'static str {
    "usage: syncplay-gui-native-smoke [--binary PATH] [--timeout-ms N] [--json|--text] [--keep-open]"
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn default_binary_path() -> PathBuf {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(parent) = current_exe.parent()
    {
        let candidate = parent.join("syncplay-gui.exe");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("target")
        .join("debug")
        .join("syncplay-gui.exe")
}

fn resolve_binary_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve syncplay-gui binary at {path:?}: {error}"))
}

fn launch_syncplay_gui(binary_path: &Path, launch: GuiLaunchConfig<'_>) -> Result<Child, String> {
    let mut command = Command::new(binary_path);
    if let Some(parent) = binary_path.parent() {
        command.current_dir(parent);
    }
    for name in [
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP",
        "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK",
        "SYNCPLAY_CLIENT_HOST",
        "SYNCPLAY_CLIENT_PORT",
        "SYNCPLAY_CLIENT_USERNAME",
        "SYNCPLAY_CLIENT_ROOM",
    ] {
        command.env_remove(name);
    }
    command.env("SYNCPLAY_CLIENT_CONFIG_PATH", launch.config_path);
    command.env(
        "SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS",
        launch.public_servers_spec,
    );
    command.env(
        "SYNCPLAY_GUI_TEST_OPEN_MEDIA_FILE_PATHS",
        launch.open_media_file_path.display().to_string(),
    );
    command.env(
        "SYNCPLAY_GUI_TEST_MEDIA_SEARCH_BROWSE_PATH",
        launch.media_search_browse_path.display().to_string(),
    );
    if let Some(tcp_session) = launch.tcp_session {
        command.env("SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP", "true");
        command.env("SYNCPLAY_CLIENT_HOST", tcp_session.host);
        command.env("SYNCPLAY_CLIENT_PORT", tcp_session.port.to_string());
        command.env("SYNCPLAY_CLIENT_USERNAME", tcp_session.username);
        command.env("SYNCPLAY_CLIENT_ROOM", tcp_session.room);
    } else if let Some((username, room)) = launch.loopback_session {
        command.env("SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK", "true");
        command.env("SYNCPLAY_CLIENT_USERNAME", username);
        command.env("SYNCPLAY_CLIENT_ROOM", room);
    }
    command
        .spawn()
        .map_err(|error| format!("failed to launch syncplay-gui at {binary_path:?}: {error}"))
}

fn wait_for_main_window<D: NativeGuiDriver>(
    driver: &D,
    child: &mut Child,
    timeout: Duration,
) -> Result<D::WindowHandle, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to poll syncplay-gui process state: {error}"))?
        {
            return Err(format!(
                "syncplay-gui exited before exposing a main window (status: {status})"
            ));
        }

        if let Some(window) = driver.find_main_window(child.id())? {
            return Ok(window);
        }

        if Instant::now() >= deadline {
            return Err("timed out waiting for the syncplay-gui main window".to_owned());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_process_exit(child: &mut Child, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll syncplay-gui exit state: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                "timed out waiting for syncplay-gui to exit after close request".to_owned(),
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn seed_native_smoke_config(config_path: &Path) -> Result<(), String> {
    let settings = StoredClientSettingsMvp {
        host: Some(CONFIG_HOST_VALUE.to_owned()),
        port: Some(CONFIG_PORT_VALUE.parse().unwrap()),
        username: Some(CONFIG_USERNAME_VALUE.to_owned()),
        room: Some(CONFIG_ROOM_VALUE.to_owned()),
        player_path: Some(CONFIG_PLAYER_PATH_VALUE.to_owned()),
        folder_search_first_file_timeout_seconds: Some(MEDIA_SEARCH_FIRST_FILE_TIMEOUT_SECONDS),
        folder_search_timeout_seconds: Some(MEDIA_SEARCH_TIMEOUT_SECONDS),
        folder_search_double_check_interval_seconds: Some(
            MEDIA_SEARCH_DOUBLE_CHECK_INTERVAL_SECONDS,
        ),
        folder_search_warning_threshold_seconds: Some(MEDIA_SEARCH_WARNING_THRESHOLD_SECONDS),
        ..StoredClientSettingsMvp::default()
    };
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(config_path, &settings).map_err(
        |error| {
            format!(
                "failed to seed native smoke config {}: {error}",
                config_path.display()
            )
        },
    )
}

fn launch_syncplay_gui_with_retry<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    launch: GuiLaunchConfig<'_>,
    timeout: Duration,
) -> Result<(Child, D::WindowHandle), String> {
    let mut last_error = String::new();
    for attempt in 1..=LAUNCH_ATTEMPTS {
        let mut child = launch_syncplay_gui(binary_path, launch)?;
        match wait_for_main_window(driver, &mut child, timeout) {
            Ok(window) => {
                let _ = driver.prepare_window_for_smoke(window);
                return Ok((child, window));
            }
            Err(error) => {
                let retryable = child
                    .try_wait()
                    .ok()
                    .flatten()
                    .and_then(|status| status.code())
                    .is_some_and(|status| status as u32 == DLL_INIT_FAILED_STATUS);
                last_error = error;
                let _ = child.kill();
                let _ = child.wait();
                if retryable && attempt < LAUNCH_ATTEMPTS {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
                break;
            }
        }
    }
    Err(last_error)
}

fn wait_for_file_contains(
    path: &Path,
    expected_snippets: &[&str],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_contents = String::new();
    loop {
        match fs::read_to_string(path) {
            Ok(contents) => {
                if expected_snippets
                    .iter()
                    .all(|snippet| contents.contains(snippet))
                {
                    return Ok(());
                }
                last_contents = contents;
            }
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting for config file {path:?} to contain required lines; last read error: {error}"
                    ));
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for config file {:?} to contain [{}]. Last file contents:\n{}",
                path,
                expected_snippets
                    .iter()
                    .map(|snippet| format!("{snippet:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                last_contents
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn normalize_menu_label(raw_label: &str) -> String {
    raw_label.replace('&', "").trim().to_owned()
}

fn verify_menu_contract(menu_labels: &[String]) -> Result<(), String> {
    let normalized = menu_labels
        .iter()
        .map(|label| normalize_menu_label(label))
        .collect::<Vec<_>>();
    let required = ["File", "Playback", "Advanced", "Window", "Help"];
    for expected in required {
        if !normalized.iter().any(|label| label == expected) {
            return Err(format!(
                "main window menu is missing required top-level entry {expected:?}; observed: {}",
                normalized.join(", ")
            ));
        }
    }
    Ok(())
}

fn verify_accessibility_contract(accessible_names: &[String]) -> Result<(), String> {
    if accessible_names.is_empty() {
        return Err("accessibility tree did not expose any named elements".to_owned());
    }

    let required_labels = ["File", "Playback", "Advanced", "Window", "Help"];
    for required_label in required_labels {
        if !accessible_names.iter().any(|name| name == required_label) {
            return Err(format!(
                "accessibility tree is missing required top-level label {required_label:?}"
            ));
        }
    }

    if !accessible_names
        .iter()
        .any(|name| name == "view: configuration" || name == "view: main-window")
    {
        return Err(
            "accessibility tree is missing a known view indicator (expected 'view: configuration' or 'view: main-window')"
                .to_owned(),
        );
    }

    Ok(())
}

fn contains_accessible_name(accessible_names: &[String], expected: &str) -> bool {
    accessible_names.iter().any(|name| name == expected)
}

fn render_accessible_name_snapshot_for_patterns(
    accessible_names: &[String],
    patterns: &[&str],
) -> String {
    let snapshot = accessible_names
        .iter()
        .filter(|name| patterns.iter().any(|pattern| name.contains(pattern)))
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>();
    if snapshot.is_empty() {
        "none".to_owned()
    } else {
        snapshot.join(", ")
    }
}

fn wait_for_accessible_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if contains_accessible_name(&names, expected_name) {
                    return Ok(());
                }
                last_snapshot = Some(render_accessible_name_snapshot_for_patterns(
                    &names,
                    &[
                        "view:",
                        "self=",
                        "ready=",
                        "controller=",
                        "Timeout",
                        "Warning",
                        "Interval",
                        "Media Search",
                        "view: media-search",
                    ],
                ));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for accessibility name {expected_name:?}; last accessibility read error: {error}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            } else {
                Err(format!(
                    "timed out waiting for accessibility name {expected_name:?}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_any_accessible_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_names: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if let Some(found) = expected_names
                    .iter()
                    .find(|expected| contains_accessible_name(&names, expected))
                {
                    return Ok((*found).to_owned());
                }
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            let expected_list = expected_names
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for one of [{expected_list}] in accessibility tree; last accessibility read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for one of [{expected_list}] in accessibility tree"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn invoke_named_control_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_snapshot = None;
    loop {
        match driver.invoke_named_control(window, name, control_kind) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    let snapshot = driver
                        .accessible_names(window)
                        .map(|names| {
                            render_accessible_name_snapshot_for_patterns(
                                &names,
                                &[name, "Save", "Reset", "Reload", "Configuration", "view:"],
                            )
                        })
                        .unwrap_or_else(|_| "unavailable".to_owned());
                    return Err(format!(
                        "timed out invoking {} named {name:?}; last error: {error}; last snapshot: {}",
                        control_kind.label(),
                        if last_snapshot.is_some() {
                            last_snapshot.take().unwrap()
                        } else {
                            snapshot
                        }
                    ));
                }
                last_snapshot = driver.accessible_names(window).ok().map(|names| {
                    render_accessible_name_snapshot_for_patterns(
                        &names,
                        &[name, "Save", "Reset", "Reload", "Configuration", "view:"],
                    )
                });
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn invoke_menu_command_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    menu_name: &str,
    command_name: &str,
    command_kind: NativeControlKind,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let _ = driver.invoke_named_control(window, menu_name, NativeControlKind::Any);
        thread::sleep(Duration::from_millis(100));
        match driver.invoke_named_control(window, command_name, command_kind) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out invoking menu command {menu_name:?}->{command_name:?}; last error: {error}"
                    ));
                }
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
}

fn wait_for_named_control_count<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    loop {
        match driver.count_named_controls(window, name, control_kind) {
            Ok(count) if count == expected_count => return Ok(()),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for {expected_count} controls named {name:?}; last count error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for {expected_count} controls named {name:?}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

impl MockSessionServer {
    fn recv_hello(&self, timeout: Duration, label: &str) -> Result<String, String> {
        self.hello_rx.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} hello line from mock TCP server: {error}")
        })
    }

    fn recv_chat(&self, timeout: Duration, label: &str) -> Result<String, String> {
        self.chat_rx.recv_timeout(timeout).map_err(|error| {
            format!("timed out waiting for {label} chat line from mock TCP server: {error}")
        })
    }

    fn release(mut self, label: &str) -> Result<(), String> {
        let _ = self.release_tx.send(());
        let Some(join_handle) = self.join_handle.take() else {
            return Ok(());
        };
        match join_handle.join() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(format!("{label} mock TCP server failed: {error}")),
            Err(_) => Err(format!("{label} mock TCP server thread panicked")),
        }
    }
}

fn start_mock_session_server(
    initial_lines: &'static [&'static str],
    first_chat_followup_lines: &'static [&'static str],
    second_chat_followup_lines: &'static [&'static str],
) -> Result<MockSessionServer, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind mock TCP listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set mock TCP listener nonblocking mode: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read mock TCP listener address: {error}"))?;
    let port = address.port();

    let (hello_tx, hello_rx) = mpsc::channel();
    let (chat_tx, chat_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let join_handle = thread::spawn(move || -> Result<(), String> {
        let accept_deadline = Instant::now() + Duration::from_secs(25);
        let (mut stream, _) = loop {
            if release_rx.try_recv().is_ok() {
                return Ok(());
            }
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() >= accept_deadline {
                        return Err(
                            "mock TCP server timed out waiting for client connection".to_owned()
                        );
                    }
                    thread::sleep(Duration::from_millis(40));
                    continue;
                }
                Err(error) => {
                    return Err(format!("mock TCP server failed to accept client: {error}"));
                }
            }
        };
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("mock TCP server failed to restore blocking mode: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("mock TCP server failed to set read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("mock TCP server failed to set write timeout: {error}"))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("mock TCP server failed to clone stream: {error}"))?;
        let mut reader = BufReader::new(reader_stream);
        let mut hello_line = String::new();
        reader.read_line(&mut hello_line).map_err(|error| {
            format!("mock TCP server failed to read startup hello line: {error}")
        })?;
        hello_tx.send(hello_line).map_err(|error| {
            format!("mock TCP server failed to report startup hello line: {error}")
        })?;
        for line in initial_lines {
            stream
                .write_all(line.as_bytes())
                .map_err(|error| format!("mock TCP server failed to write state line: {error}"))?;
            stream.write_all(b"\n").map_err(|error| {
                format!("mock TCP server failed to terminate state line: {error}")
            })?;
        }

        let mut process_followup = |phase_label: &str,
                                    lines: &'static [&'static str]|
         -> Result<(), String> {
            if lines.is_empty() {
                return Ok(());
            }

            let mut chat_line = String::new();
            reader.read_line(&mut chat_line).map_err(|error| {
                format!("mock TCP server failed to read {phase_label} chat line: {error}")
            })?;
            if chat_line.trim().is_empty() {
                return Err(format!(
                    "mock TCP server received an empty {phase_label} chat line"
                ));
            }
            chat_tx.send(chat_line).map_err(|error| {
                format!("mock TCP server failed to report {phase_label} chat line: {error}")
            })?;

            for line in lines {
                if let Err(error) = stream.write_all(line.as_bytes()) {
                    if matches!(
                        error.kind(),
                        ErrorKind::BrokenPipe
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::ConnectionReset
                    ) {
                        break;
                    }
                    return Err(format!(
                        "mock TCP server failed to write {phase_label} follow-up state line: {error}"
                    ));
                }
                if let Err(error) = stream.write_all(b"\n") {
                    if matches!(
                        error.kind(),
                        ErrorKind::BrokenPipe
                            | ErrorKind::ConnectionAborted
                            | ErrorKind::ConnectionReset
                    ) {
                        break;
                    }
                    return Err(format!(
                        "mock TCP server failed to terminate {phase_label} follow-up state line: {error}"
                    ));
                }
            }
            Ok(())
        };

        process_followup("first", first_chat_followup_lines)?;
        process_followup("second", second_chat_followup_lines)?;

        let _ = release_rx.recv_timeout(Duration::from_secs(10));
        Ok(())
    });

    Ok(MockSessionServer {
        address: address.to_string(),
        port,
        hello_rx,
        chat_rx,
        release_tx,
        join_handle: Some(join_handle),
    })
}

fn verify_interaction_contract<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    config_path: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let step_timeout = timeout.min(Duration::from_millis(4_000));
    let config_persist_timeout = timeout.min(Duration::from_millis(8_000));
    let mut steps = Vec::new();

    let initial_view = wait_for_any_accessible_name(
        driver,
        window,
        &["view: configuration", "view: main-window"],
        step_timeout,
    )?;
    if initial_view == "view: main-window" {
        invoke_named_control_with_wait(
            driver,
            window,
            "Configuration",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: configuration", step_timeout)?;
    }
    let editable_count = driver.editable_text_input_count(window)?;
    if editable_count < 6 {
        return Err(format!(
            "expected at least 6 editable configuration text fields, found {editable_count}"
        ));
    }
    for (edit_index, expected_value) in [
        (0usize, CONFIG_HOST_VALUE),
        (1usize, CONFIG_PORT_VALUE),
        (2usize, CONFIG_USERNAME_VALUE),
        (3usize, CONFIG_ROOM_VALUE),
        (5usize, CONFIG_PLAYER_PATH_VALUE),
    ] {
        let current_value = driver.get_edit_value_by_index(window, edit_index)?;
        if current_value != expected_value {
            driver.set_edit_value_by_index(window, edit_index, expected_value)?;
        }
    }
    driver.set_edit_value_by_index(window, TRUSTED_DOMAINS_EDIT_INDEX, TRUSTED_DOMAINS_VALUE)?;
    wait_for_edit_value_by_index(
        driver,
        window,
        TRUSTED_DOMAINS_EDIT_INDEX,
        TRUSTED_DOMAINS_VALUE,
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Trusted Domains Only",
        NativeControlKind::Any,
        step_timeout,
    )?;
    let password_value = driver.get_edit_value_by_index(window, 4)?;
    if !password_value.is_empty()
        && let Err(error) = driver.set_edit_value_by_index(window, 4, "")
    {
        steps.push(format!(
            "config-password-set-skipped:{}",
            error.replace('|', "/").replace('\n', " ")
        ));
    }
    if let Err(error) =
        wait_for_edit_value_by_index(driver, window, 0, CONFIG_HOST_VALUE, step_timeout)
    {
        steps.push(format!(
            "config-host-verify-skipped:{}",
            error.replace('|', "/").replace('\n', " ")
        ));
    }
    for (edit_index, expected_value) in [
        (1usize, CONFIG_PORT_VALUE),
        (2usize, CONFIG_USERNAME_VALUE),
        (3usize, CONFIG_ROOM_VALUE),
        (5usize, CONFIG_PLAYER_PATH_VALUE),
    ] {
        wait_for_edit_value_by_index(driver, window, edit_index, expected_value, step_timeout)?;
    }

    invoke_named_control_with_wait(
        driver,
        window,
        "Save",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "pending: save-configuration", step_timeout)?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Complete",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(
        driver,
        window,
        "success: Configuration saved.",
        step_timeout,
    )?;
    let config_persist_result = wait_for_file_contains(
        config_path,
        &[
            "host = syncplay.example",
            "port = 8999",
            "name = smoke-user",
            "room = smoke-room",
            "playerPath = C:\\Windows\\System32\\notepad.exe",
            "onlySwitchToTrustedDomains = True",
            "trustedDomains = ['youtube.com', '*.example.com/videos']",
        ],
        config_persist_timeout,
    );
    match config_persist_result {
        Ok(()) => {
            steps.push("config-save-persisted".to_owned());
            steps.push("trusted-domains-configured".to_owned());
        }
        Err(error) => steps.push(format!(
            "config-save-persisted-skipped:{}",
            error.replace('|', "/").replace('\n', " ")
        )),
    }

    wait_for_accessible_name(driver, window, "Public Servers", step_timeout)?;
    wait_for_accessible_name(driver, window, "2", step_timeout)?;

    invoke_named_control_with_wait(
        driver,
        window,
        "Public Servers",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: public-servers", step_timeout)?;
    steps.push("surface-public-servers".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Alpha: alpha.example:8999",
        NativeControlKind::Any,
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Connect",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(
        driver,
        window,
        "pending: connect-public-server",
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Complete",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, CONNECT_NO_SESSION_ERROR, step_timeout)?;
    steps.push("public-server-connect-error".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Refresh",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(
        driver,
        window,
        "pending: refresh-public-servers",
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Complete",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, REFRESH_NO_SESSION_ERROR, step_timeout)?;
    steps.push("public-server-refresh-error".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Add Custom Server",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "Edit Session", step_timeout)?;
    let edit_count = driver.editable_text_input_count(window)?;
    if edit_count != 2 {
        return Err(format!(
            "expected 2 editable public-server edit-session fields, found {edit_count}"
        ));
    }
    driver.set_edit_value_by_index(window, 0, CUSTOM_SERVER_LABEL)?;
    driver.set_edit_value_by_index(window, 1, CUSTOM_SERVER_ADDRESS)?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Save Changes",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_named_control_count(
        driver,
        window,
        "Save Changes",
        NativeControlKind::Button,
        0,
        step_timeout,
    )?;
    let custom_row_name = wait_for_any_accessible_name(
        driver,
        window,
        &[CUSTOM_SERVER_ROW_NAME, CUSTOM_SERVER_LABEL],
        step_timeout,
    )?;
    if custom_row_name == CUSTOM_SERVER_LABEL {
        wait_for_accessible_name(driver, window, CUSTOM_SERVER_ADDRESS, step_timeout)?;
    }
    steps.push("public-server-add-custom".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        &custom_row_name,
        NativeControlKind::Any,
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Connect",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(
        driver,
        window,
        "pending: connect-public-server",
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Complete",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, CONNECT_NO_SESSION_ERROR, step_timeout)?;
    steps.push("public-server-connect-custom-pending".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Configuration",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: configuration", step_timeout)?;
    for (index, expected_value) in [(0usize, CUSTOM_SERVER_HOST), (1usize, CUSTOM_SERVER_PORT)] {
        let actual = driver.get_edit_value_by_index(window, index)?;
        if actual != expected_value {
            return Err(format!(
                "custom public-server selection did not update configuration edit field [{index}]: expected {expected_value:?}, got {actual:?}"
            ));
        }
    }
    steps.push("public-server-custom-applied".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Media Search",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: media-search", step_timeout)?;
    steps.push("surface-media-search".to_owned());
    wait_for_accessible_name(driver, window, "First File Timeout: 3.00s", step_timeout)?;
    wait_for_accessible_name(driver, window, "Search Timeout: 30.00s", step_timeout)?;
    let mut double_check_visible =
        wait_for_accessible_name(driver, window, "Double Check Interval: 2.50s", step_timeout)
            .is_ok();
    let mut warning_threshold_visible =
        wait_for_accessible_name(driver, window, "Warning Threshold: 7.50s", step_timeout).is_ok();
    let mut page_down_count = 0usize;
    let timing_retry_timeout = step_timeout.min(Duration::from_millis(1_000));
    while page_down_count < 2 && (!double_check_visible || !warning_threshold_visible) {
        let _ = driver.scroll_active_view_page_down(window);
        page_down_count += 1;
        if !double_check_visible {
            double_check_visible = wait_for_accessible_name(
                driver,
                window,
                "Double Check Interval: 2.50s",
                timing_retry_timeout,
            )
            .is_ok();
        }
        if !warning_threshold_visible {
            warning_threshold_visible = wait_for_accessible_name(
                driver,
                window,
                "Warning Threshold: 7.50s",
                timing_retry_timeout,
            )
            .is_ok();
        }
    }
    for _ in 0..page_down_count {
        let _ = driver.scroll_active_view_page_up(window);
    }
    if double_check_visible && warning_threshold_visible {
        steps.push("media-search-timing-visible".to_owned());
        steps.push("media-search-timing-values-visible".to_owned());
    } else {
        return Err(format!(
            "media-search timing values were not all visible: first_file=yes, search=yes, double_check={}, warning_threshold={}",
            bool_label(double_check_visible),
            bool_label(warning_threshold_visible)
        ));
    }

    invoke_named_control_with_wait(
        driver,
        window,
        "Browse Directories",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(
        driver,
        window,
        &media_search_browse_path.display().to_string(),
        step_timeout,
    )?;
    steps.push("media-search-browse".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Browse Directories",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_named_control_count(
        driver,
        window,
        &media_search_browse_path.display().to_string(),
        NativeControlKind::Any,
        1,
        step_timeout,
    )?;
    steps.push("media-search-browse-dedupe".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Search Missing Media",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(
        driver,
        window,
        "pending: search-missing-media",
        step_timeout,
    )?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Complete",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, SEARCH_NO_SESSION_ERROR, step_timeout)?;
    steps.push("media-search-error".to_owned());

    invoke_named_control_with_wait(
        driver,
        window,
        "Configuration",
        NativeControlKind::Button,
        step_timeout,
    )?;
    wait_for_accessible_name(driver, window, "view: configuration", step_timeout)?;
    steps.push("surface-configuration".to_owned());

    if let Err(error) = invoke_named_control_with_wait(
        driver,
        window,
        "Shared Playlists",
        NativeControlKind::Any,
        step_timeout,
    ) {
        steps.push(format!(
            "open-media-prep-shared-playlists-skipped:{}",
            error.replace('|', "/").replace('\n', " ")
        ));
    } else {
        steps.push("open-media-prep-shared-playlists".to_owned());
    }

    let open_media_invoked = if let Err(primary_error) = invoke_menu_command_with_wait(
        driver,
        window,
        "File",
        "Open Media File",
        NativeControlKind::MenuItem,
        step_timeout,
    ) {
        match invoke_menu_command_with_wait(
            driver,
            window,
            "File",
            "Open Media File",
            NativeControlKind::Any,
            step_timeout,
        ) {
            Ok(()) => true,
            Err(fallback_error) => {
                steps.push(format!(
                    "open-media-file-skipped:{}",
                    format!("menu-item-failure={primary_error}; fallback-failure={fallback_error}")
                        .replace('|', "/")
                ));
                false
            }
        }
    } else {
        true
    };
    if open_media_invoked {
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            &open_media_file_path.display().to_string(),
            step_timeout,
        )?;
        steps.push("open-media-file".to_owned());
    }

    if let Err(primary_error) = invoke_menu_command_with_wait(
        driver,
        window,
        "Help",
        "About",
        NativeControlKind::MenuItem,
        step_timeout,
    ) {
        invoke_menu_command_with_wait(
            driver,
            window,
            "Help",
            "About",
            NativeControlKind::Any,
            step_timeout,
        )
        .map_err(|fallback_error| {
            format!(
                "failed to invoke About through menu item ({primary_error}); fallback also failed: {fallback_error}"
            )
        })?;
    }
    wait_for_accessible_name(driver, window, "About Syncplay", step_timeout)?;
    wait_for_accessible_name(driver, window, "modal: about", step_timeout)?;
    steps.push("about-open".to_owned());

    Ok(steps)
}

fn wait_for_named_edit_value<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    expected_value: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_value = None;
    let mut last_error = None;
    loop {
        match driver.get_named_edit_value(window, name) {
            Ok(value) => {
                if value == expected_value {
                    return Ok(());
                }
                last_value = Some(value);
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for edit field {name:?} to equal {expected_value:?}; last read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for edit field {name:?} to equal {expected_value:?}; last value: {last_value:?}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_edit_value_by_index<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    edit_index: usize,
    expected_value: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_value = None;
    let mut last_error = None;
    loop {
        match driver.get_edit_value_by_index(window, edit_index) {
            Ok(value) => {
                if value == expected_value {
                    return Ok(());
                }
                last_value = Some(value);
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for edit field [{edit_index}] to equal {expected_value:?}; last read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for edit field [{edit_index}] to equal {expected_value:?}; last value: {last_value:?}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn assert_chat_input_cleared<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    timeout: Duration,
) -> Result<(), String> {
    wait_for_named_edit_value(driver, window, "Chat Input", "", timeout)
}

fn wait_for_visible_chat_message<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    sender: &str,
    message: &str,
    timeout: Duration,
) -> Result<(), String> {
    let expected_label = format!("{sender}: {message}");
    wait_for_accessible_name(driver, window, &expected_label, timeout)
}

fn send_chat_message_and_complete<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    message: &str,
    timeout: Duration,
) -> Result<(), String> {
    driver.set_named_edit_value(window, "Chat Input", message, true)?;
    wait_for_accessible_name(driver, window, "pending: send-chat-message", timeout)?;
    invoke_named_control_with_wait(
        driver,
        window,
        "Complete",
        NativeControlKind::Button,
        timeout,
    )?;
    assert_chat_input_cleared(driver, window, timeout)?;
    Ok(())
}

fn assert_mock_chat_line(line: &str, expected_message: &str, label: &str) -> Result<(), String> {
    if !line.contains("\"Chat\"") {
        return Err(format!(
            "{label} mock TCP server did not receive a chat payload line: {line:?}"
        ));
    }
    if !line.contains(expected_message) {
        return Err(format!(
            "{label} mock TCP server chat payload missing expected message {expected_message:?}: {line:?}"
        ));
    }
    Ok(())
}

fn verify_relaunch_config_reload_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    config_path: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let persisted_contents = fs::read_to_string(config_path).map_err(|error| {
        format!(
            "failed reading reloaded configuration {} before relaunch: {error}",
            config_path.display()
        )
    })?;
    let persisted_settings = load_syncplay_ini_stored_client_settings_mvp_from_path(config_path)
        .map_err(|error| {
            format!(
                "failed to parse reloaded configuration {} before relaunch: {error}",
                config_path.display()
            )
        })?
        .unwrap_or_default();
    let expected_trusted_domains =
        vec!["youtube.com".to_owned(), "*.example.com/videos".to_owned()];
    if persisted_settings.host.as_deref() != Some(CONFIG_HOST_VALUE)
        || persisted_settings.port != Some(CONFIG_PORT_VALUE.parse().unwrap())
        || persisted_settings.username.as_deref() != Some(CONFIG_USERNAME_VALUE)
        || persisted_settings.room.as_deref() != Some(CONFIG_ROOM_VALUE)
        || persisted_settings.player_path.as_deref() != Some(CONFIG_PLAYER_PATH_VALUE)
        || persisted_settings.only_switch_to_trusted_domains != Some(true)
        || persisted_settings.trusted_domains.as_ref() != Some(&expected_trusted_domains)
    {
        return Err(format!(
            "reloaded configuration file did not retain saved connection/trusted-domain values before relaunch: host={:?}, port={:?}, username={:?}, room={:?}, player_path={:?}, only_switch_to_trusted_domains={:?}, trusted_domains={:?}; file contents:\n{}",
            persisted_settings.host,
            persisted_settings.port,
            persisted_settings.username,
            persisted_settings.room,
            persisted_settings.player_path,
            persisted_settings.only_switch_to_trusted_domains,
            persisted_settings.trusted_domains,
            persisted_contents,
        ));
    }
    let launch = GuiLaunchConfig {
        config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: None,
    };
    let (mut child, window) = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout)?;

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(4_000));
        let mut steps = Vec::new();

        let initial_view = wait_for_any_accessible_name(
            driver,
            window,
            &["view: configuration", "view: main-window"],
            step_timeout,
        )?;
        if initial_view == "view: main-window" {
            invoke_named_control_with_wait(
                driver,
                window,
                "Configuration",
                NativeControlKind::Button,
                step_timeout,
            )?;
        }
        wait_for_accessible_name(driver, window, "view: configuration", step_timeout)?;

        let editable_count = driver.editable_text_input_count(window)?;
        if editable_count < 6 {
            return Err(format!(
                "expected at least 6 editable configuration text fields after relaunch, found {editable_count}"
            ));
        }
        for (index, expected_value) in [
            (0usize, CONFIG_HOST_VALUE),
            (1usize, CONFIG_PORT_VALUE),
            (2usize, CONFIG_USERNAME_VALUE),
            (3usize, CONFIG_ROOM_VALUE),
            (5usize, CONFIG_PLAYER_PATH_VALUE),
        ] {
            wait_for_edit_value_by_index(driver, window, index, expected_value, step_timeout)?;
        }
        steps.push("config-reload-persisted".to_owned());
        wait_for_edit_value_by_index(
            driver,
            window,
            TRUSTED_DOMAINS_EDIT_INDEX,
            TRUSTED_DOMAINS_VALUE,
            step_timeout,
        )?;
        steps.push("trusted-domains-persisted".to_owned());

        if let Err(error) = invoke_named_control_with_wait(
            driver,
            window,
            "Shared Playlists",
            NativeControlKind::Any,
            step_timeout,
        ) {
            steps.push(format!(
                "relaunch-open-media-prep-shared-playlists-skipped:{}",
                error.replace('|', "/").replace('\n', " ")
            ));
        } else {
            steps.push("relaunch-open-media-prep-shared-playlists".to_owned());
        }

        if let Err(primary_error) = invoke_menu_command_with_wait(
            driver,
            window,
            "File",
            "Open Media File",
            NativeControlKind::MenuItem,
            step_timeout,
        ) && let Err(fallback_error) = invoke_menu_command_with_wait(
            driver,
            window,
            "File",
            "Open Media File",
            NativeControlKind::Any,
            step_timeout,
        ) {
            invoke_named_control_with_wait(
                driver,
                window,
                "Open Media File",
                NativeControlKind::Any,
                step_timeout,
            )
            .map_err(|control_error| {
                format!(
                    "failed to invoke File->Open Media File after relaunch through menu item ({primary_error}); menu fallback failed ({fallback_error}); direct control fallback also failed: {control_error}"
                )
            })?;
        }
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            &open_media_file_path.display().to_string(),
            step_timeout,
        )?;
        steps.push("relaunch-open-media-file".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

fn verify_loopback_chat_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    config_path: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let launch = GuiLaunchConfig {
        config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: Some((TRANSPORT_SESSION_USERNAME, TRANSPORT_SESSION_ROOM)),
    };
    let (mut child, window) = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout)?;

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(6_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(
            driver,
            window,
            &["view: configuration", "view: main-window"],
            step_timeout,
        )?;
        let _ = invoke_named_control_with_wait(
            driver,
            window,
            "Main Window",
            NativeControlKind::Button,
            step_timeout,
        );
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;

        send_chat_message_and_complete(driver, window, "helloloopback", step_timeout)?;
        steps.push("loopback-chat-send".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

fn verify_live_python_peer_connect_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let mut python_harness = LegacyServerPythonPeerHarness::spawn(
        LIVE_PYTHON_INTEROP_PEER_USERNAME,
        LIVE_PYTHON_INTEROP_ROOM,
    )
    .map_err(|error| format!("failed to start live Python interop harness: {error}"))?;
    let interop_config_path = temp_root.join("syncplay-native-smoke-python-interop.ini");
    let _ = fs::remove_file(&interop_config_path);
    seed_native_smoke_config(&interop_config_path)?;
    let launch = GuiLaunchConfig {
        config_path: &interop_config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: Some(TcpSessionBootstrap {
            host: python_harness.host(),
            port: python_harness.port(),
            username: LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            room: LIVE_PYTHON_INTEROP_ROOM,
        }),
        loopback_session: None,
    };

    let launch_result = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let release = python_harness.shutdown();
            let mut combined_error =
                format!("failed to launch live Python interop segment for native smoke: {error}");
            if let Err(release_error) = release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error.to_string());
            }
            return Err(combined_error);
        }
    };

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(8_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(
            driver,
            window,
            &["view: configuration", "view: main-window"],
            step_timeout,
        )?;
        let _ = invoke_named_control_with_wait(
            driver,
            window,
            "Main Window",
            NativeControlKind::Button,
            step_timeout,
        );
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Configuration",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: configuration", step_timeout)?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Main Window",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_ROW_NAME,
            step_timeout,
        )?;
        python_harness
            .start_peer_connected()
            .map_err(|error| format!("failed to connect live Python reference peer: {error}"))?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Configuration",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: configuration", step_timeout)?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Main Window",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_ROW_NAME,
            step_timeout,
        )?;
        steps.push("transport-python-peer-connect".to_owned());

        invoke_named_control_with_wait(
            driver,
            window,
            "Set Ready",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_READY_ROW_NAME,
            step_timeout,
        )?;
        python_harness
            .wait_for_peer_observed_user_ready(
                LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
                true,
                step_timeout,
            )
            .map_err(|error| {
                format!("python reference peer did not observe local ready=true: {error}")
            })?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Set Ready",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_ROW_NAME,
            step_timeout,
        )?;
        python_harness
            .wait_for_peer_observed_user_ready(
                LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
                false,
                step_timeout,
            )
            .map_err(|error| {
                format!("python reference peer did not observe local ready=false: {error}")
            })?;
        python_harness
            .set_peer_ready(true)
            .map_err(|error| format!("failed to set Python reference peer ready=true: {error}"))?;
        python_harness
            .wait_for_peer_local_ready(true, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm ready=true: {error}")
            })?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_READY_ROW_NAME,
            step_timeout,
        )?;
        python_harness
            .set_peer_ready(false)
            .map_err(|error| format!("failed to set Python reference peer ready=false: {error}"))?;
        python_harness
            .wait_for_peer_local_ready(false, step_timeout)
            .map_err(|error| {
                format!("python reference peer did not confirm ready=false: {error}")
            })?;
        wait_for_accessible_name(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_ROW_NAME,
            step_timeout,
        )?;
        steps.push("transport-python-peer-readiness".to_owned());

        send_chat_message_and_complete(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
            step_timeout,
        )?;
        wait_for_visible_chat_message(
            driver,
            window,
            LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
            LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
            step_timeout,
        )?;
        python_harness
            .wait_for_peer_observed_chat_message(
                LIVE_PYTHON_INTEROP_LOCAL_USERNAME,
                LIVE_PYTHON_INTEROP_LOCAL_CHAT_MESSAGE,
                step_timeout,
            )
            .map_err(|error| {
                format!("python reference peer did not observe local chat message: {error}")
            })?;
        steps.push("transport-python-peer-chat-local-to-peer".to_owned());

        python_harness
            .send_peer_chat_message(LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE)
            .map_err(|error| format!("failed to send Python reference peer chat: {error}"))?;
        python_harness
            .wait_for_peer_observed_chat_message(
                LIVE_PYTHON_INTEROP_PEER_USERNAME,
                LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
                step_timeout,
            )
            .map_err(|error| {
                format!("python reference peer did not confirm its own chat echo: {error}")
            })?;
        wait_for_visible_chat_message(
            driver,
            window,
            LIVE_PYTHON_INTEROP_PEER_USERNAME,
            LIVE_PYTHON_INTEROP_PEER_CHAT_MESSAGE,
            step_timeout,
        )?;
        steps.push("transport-python-peer-chat-peer-to-local".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let release = python_harness.shutdown();
    match (outcome, release) {
        (Ok(steps), Ok(())) => Ok(steps),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error.to_string()),
        (Err(error), Err(release_error)) => Err(format!("{error}; {release_error}")),
    }
}

fn verify_transport_reconnect_contract<D: NativeGuiDriver>(
    driver: &D,
    binary_path: &Path,
    temp_root: &Path,
    media_search_browse_path: &Path,
    open_media_file_path: &Path,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let primary_server = start_mock_session_server(
        &[
            r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["episode1.mkv","episode2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":10.0,"paused":true,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"smoke-room"},"file":{"name":"bob.mp4"},"isReady":true,"controller":true}}}}"#,
        ],
        &[
            r#"{"Chat":{"username":"smoke-user","message":"hellotcp"}}"#,
            r#"{"Set":{"playlistChange":{"files":["postchat1.mkv","postchat2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"bob":{"room":{"name":"smoke-room"},"file":{"name":"bob-post.mp4"},"isReady":false,"controller":false}}}}"#,
        ],
        &[
            r#"{"Chat":{"username":"smoke-user","message":"goodbyeprimary"}}"#,
            r#"{"Set":{"user":{"bob":{"event":{"left":true}}}}}"#,
        ],
    )?;
    let reconnect_server = start_mock_session_server(
        &[
            r#"{"Hello":{"username":"smoke-user","room":{"name":"smoke-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect1.mkv","reconnect2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":false,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":20.0,"paused":false,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"smoke-room"},"file":{"name":"carol.mp4"},"isReady":false,"controller":false}}}}"#,
        ],
        &[
            r#"{"Chat":{"username":"smoke-user","message":"helloreconnect"}}"#,
            r#"{"Set":{"playlistChange":{"files":["reconnect-post1.mkv","reconnect-post2.mkv"],"user":"smoke-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"smoke-user"}}}"#,
            r#"{"Set":{"ready":{"isReady":true,"username":"smoke-user"}}}"#,
            r#"{"State":{"playstate":{"position":30.0,"paused":true,"doSeek":false,"setBy":"smoke-user"}}}"#,
            r#"{"Set":{"user":{"carol":{"room":{"name":"smoke-room"},"file":{"name":"carol-post.mp4"},"isReady":true,"controller":true}}}}"#,
        ],
        &[
            r#"{"Chat":{"username":"smoke-user","message":"goodbyereconnect"}}"#,
            r#"{"Set":{"user":{"carol":{"event":{"left":true}}}}}"#,
        ],
    )?;

    let transport_config_path = temp_root.join("syncplay-native-smoke-transport.ini");
    let _ = fs::remove_file(&transport_config_path);
    let public_servers_spec = format!(
        "[['Primary', '{}'], ['Reconnect', '{}']]",
        primary_server.address, reconnect_server.address
    );
    let launch = GuiLaunchConfig {
        config_path: &transport_config_path,
        media_search_browse_path,
        open_media_file_path,
        public_servers_spec: &public_servers_spec,
        tcp_session: Some(TcpSessionBootstrap {
            host: "127.0.0.1",
            port: primary_server.port,
            username: TRANSPORT_SESSION_USERNAME,
            room: TRANSPORT_SESSION_ROOM,
        }),
        loopback_session: None,
    };

    let launch_result = launch_syncplay_gui_with_retry(driver, binary_path, launch, timeout);
    let (mut child, window) = match launch_result {
        Ok(pair) => pair,
        Err(error) => {
            let primary_release = primary_server.release("primary");
            let reconnect_release = reconnect_server.release("reconnect");
            let mut combined_error =
                format!("failed to launch transport parity segment for native smoke: {error}");
            if let Err(release_error) = primary_release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error);
            }
            if let Err(release_error) = reconnect_release {
                combined_error.push_str("; ");
                combined_error.push_str(&release_error);
            }
            return Err(combined_error);
        }
    };

    let outcome = (|| -> Result<Vec<String>, String> {
        let step_timeout = timeout.min(Duration::from_millis(8_000));
        let mut steps = Vec::new();

        wait_for_any_accessible_name(
            driver,
            window,
            &["view: configuration", "view: main-window"],
            step_timeout,
        )?;
        let _ = invoke_named_control_with_wait(
            driver,
            window,
            "Main Window",
            NativeControlKind::Button,
            step_timeout,
        );
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;

        let first_hello = primary_server.recv_hello(step_timeout, "primary")?;
        if !first_hello.contains("\"Hello\"") {
            return Err(format!(
                "primary mock TCP server did not receive an expected startup hello payload: {first_hello:?}"
            ));
        }
        wait_for_accessible_name(driver, window, "episode2.mkv", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            "bob: self=no, ready=yes, controller=yes",
            step_timeout,
        )?;
        steps.push("transport-tcp-startup".to_owned());

        send_chat_message_and_complete(driver, window, "hellotcp", step_timeout)?;
        let first_primary_chat = primary_server.recv_chat(step_timeout, "primary first")?;
        assert_mock_chat_line(&first_primary_chat, "hellotcp", "primary first")?;
        wait_for_accessible_name(driver, window, "postchat2.mkv", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            "bob: self=no, ready=no, controller=no",
            step_timeout,
        )?;
        steps.push("transport-primary-post-chat-churn".to_owned());

        send_chat_message_and_complete(driver, window, "goodbyeprimary", step_timeout)?;
        let second_primary_chat = primary_server.recv_chat(step_timeout, "primary second")?;
        assert_mock_chat_line(&second_primary_chat, "goodbyeprimary", "primary second")?;
        wait_for_named_control_count(
            driver,
            window,
            "bob: self=no, ready=no, controller=no",
            NativeControlKind::Any,
            0,
            step_timeout,
        )?;
        steps.push("transport-primary-user-left".to_owned());

        invoke_named_control_with_wait(
            driver,
            window,
            "Public Servers",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: public-servers", step_timeout)?;
        invoke_named_control_with_wait(
            driver,
            window,
            &format!("Reconnect: {}", reconnect_server.address),
            NativeControlKind::Any,
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Connect",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(
            driver,
            window,
            "pending: connect-public-server",
            step_timeout,
        )?;
        invoke_named_control_with_wait(
            driver,
            window,
            "Complete",
            NativeControlKind::Button,
            step_timeout,
        )?;

        let second_hello = reconnect_server.recv_hello(step_timeout, "reconnect")?;
        if !second_hello.contains("\"Hello\"") {
            return Err(format!(
                "reconnect mock TCP server did not receive an expected reconnect hello payload: {second_hello:?}"
            ));
        }
        invoke_named_control_with_wait(
            driver,
            window,
            "Main Window",
            NativeControlKind::Button,
            step_timeout,
        )?;
        wait_for_accessible_name(driver, window, "view: main-window", step_timeout)?;
        wait_for_accessible_name(driver, window, "reconnect2.mkv", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            "carol: self=no, ready=no, controller=no",
            step_timeout,
        )?;
        steps.push("transport-public-server-reconnect".to_owned());

        send_chat_message_and_complete(driver, window, "helloreconnect", step_timeout)?;
        let first_reconnect_chat = reconnect_server.recv_chat(step_timeout, "reconnect first")?;
        assert_mock_chat_line(&first_reconnect_chat, "helloreconnect", "reconnect first")?;
        wait_for_accessible_name(driver, window, "reconnect-post2.mkv", step_timeout)?;
        wait_for_accessible_name(
            driver,
            window,
            "carol: self=no, ready=yes, controller=yes",
            step_timeout,
        )?;
        steps.push("transport-reconnect-post-chat-churn".to_owned());

        send_chat_message_and_complete(driver, window, "goodbyereconnect", step_timeout)?;
        let second_reconnect_chat = reconnect_server.recv_chat(step_timeout, "reconnect second")?;
        assert_mock_chat_line(
            &second_reconnect_chat,
            "goodbyereconnect",
            "reconnect second",
        )?;
        wait_for_named_control_count(
            driver,
            window,
            "carol: self=no, ready=yes, controller=yes",
            NativeControlKind::Any,
            0,
            step_timeout,
        )?;
        steps.push("transport-reconnect-user-left".to_owned());

        driver.close_window(window)?;
        wait_for_process_exit(&mut child, timeout)?;
        Ok(steps)
    })();

    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let primary_release = primary_server.release("primary");
    let reconnect_release = reconnect_server.release("reconnect");
    let mut release_errors = Vec::new();
    if let Err(error) = primary_release {
        release_errors.push(error);
    }
    if let Err(error) = reconnect_release {
        release_errors.push(error);
    }

    match outcome {
        Ok(steps) if release_errors.is_empty() => Ok(steps),
        Ok(_) => Err(release_errors.join("; ")),
        Err(error) if release_errors.is_empty() => Err(error),
        Err(error) => Err(format!("{error}; {}", release_errors.join("; "))),
    }
}

fn run_native_smoke(options: &NativeSmokeOptions) -> Result<NativeSmokeReport, String> {
    let configured_binary_path = options
        .binary_path
        .clone()
        .unwrap_or_else(default_binary_path);
    let binary_path = resolve_binary_path(&configured_binary_path)?;
    if !binary_path.exists() {
        return Err(format!(
            "syncplay-gui binary does not exist: {binary_path:?}"
        ));
    }

    let temp_root = std::env::temp_dir().join(format!(
        "syncplay-gui-native-smoke-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("failed to create native smoke temp directory: {error}"))?;
    let config_path = temp_root.join("syncplay-native-smoke.ini");
    let media_search_browse_path = temp_root.join("media-search");
    let open_media_file_path = temp_root.join("open-target.mkv");
    let _ = fs::remove_file(&config_path);
    fs::create_dir_all(&media_search_browse_path)
        .map_err(|error| format!("failed to create native smoke media directory: {error}"))?;
    fs::write(&open_media_file_path, b"open-target")
        .map_err(|error| format!("failed to create native smoke media file: {error}"))?;
    seed_native_smoke_config(&config_path)?;

    let started_at = Instant::now();
    let driver = PlatformNativeGuiDriver;
    let launch = GuiLaunchConfig {
        config_path: &config_path,
        media_search_browse_path: &media_search_browse_path,
        open_media_file_path: &open_media_file_path,
        public_servers_spec: DEFAULT_PUBLIC_SERVERS_SPEC,
        tcp_session: None,
        loopback_session: None,
    };
    let (mut child, window) =
        launch_syncplay_gui_with_retry(&driver, &binary_path, launch, options.timeout)?;
    let pid = child.id();

    let result = (|| {
        let window_title = driver.window_title(window)?;
        if !window_title.contains("Syncplay") {
            return Err(format!(
                "main window title did not match expected prefix; got {window_title:?}"
            ));
        }

        let accessible_names = driver.accessible_names(window)?;
        verify_accessibility_contract(&accessible_names)?;
        let mut interaction_steps = verify_interaction_contract(
            &driver,
            window,
            &config_path,
            &media_search_browse_path,
            &open_media_file_path,
            options.timeout,
        )?;
        let interaction_contract = "verified".to_owned();

        let menu_labels = driver.top_level_menu_labels(window)?;
        let menu_contract = if menu_labels.is_empty() {
            "skipped-no-native-menu".to_owned()
        } else {
            verify_menu_contract(&menu_labels)?;
            "verified".to_owned()
        };
        let accessibility_contract = "verified".to_owned();

        if options.keep_open {
            return Ok(NativeSmokeReport {
                binary_path: binary_path.display().to_string(),
                pid,
                window_title,
                menu_labels,
                menu_contract,
                accessible_name_count: accessible_names.len(),
                accessibility_contract,
                interaction_steps,
                interaction_contract,
                duration_ms: started_at.elapsed().as_millis(),
                closed: false,
            });
        }

        let close_step_timeout = options.timeout.min(Duration::from_millis(4_000));
        let closed_via_file_exit = if let Err(primary_error) = invoke_menu_command_with_wait(
            &driver,
            window,
            "File",
            "Exit",
            NativeControlKind::MenuItem,
            close_step_timeout,
        ) {
            match invoke_menu_command_with_wait(
                &driver,
                window,
                "File",
                "Exit",
                NativeControlKind::Any,
                close_step_timeout,
            ) {
                Ok(()) => {
                    wait_for_process_exit(&mut child, options.timeout)?;
                    interaction_steps.push("file-exit".to_owned());
                    true
                }
                Err(fallback_error) => {
                    interaction_steps.push(format!(
                        "file-exit-skipped:{}",
                        format!(
                            "menu-item-failure={primary_error}; fallback-failure={fallback_error}"
                        )
                        .replace('|', "/")
                        .replace('\n', " ")
                    ));
                    false
                }
            }
        } else {
            wait_for_process_exit(&mut child, options.timeout)?;
            interaction_steps.push("file-exit".to_owned());
            true
        };
        if !closed_via_file_exit {
            driver.close_window(window)?;
            wait_for_process_exit(&mut child, options.timeout)?;
            interaction_steps.push("window-close-fallback".to_owned());
        }

        let relaunch_steps = verify_relaunch_config_reload_contract(
            &driver,
            &binary_path,
            &config_path,
            &media_search_browse_path,
            &open_media_file_path,
            options.timeout,
        )?;
        interaction_steps.extend(relaunch_steps);

        let loopback_steps = verify_loopback_chat_contract(
            &driver,
            &binary_path,
            &config_path,
            &media_search_browse_path,
            &open_media_file_path,
            options.timeout,
        )?;
        interaction_steps.extend(loopback_steps);

        let live_python_interop_steps = verify_live_python_peer_connect_contract(
            &driver,
            &binary_path,
            &temp_root,
            &media_search_browse_path,
            &open_media_file_path,
            options.timeout,
        )?;
        interaction_steps.extend(live_python_interop_steps);

        let transport_steps = verify_transport_reconnect_contract(
            &driver,
            &binary_path,
            &temp_root,
            &media_search_browse_path,
            &open_media_file_path,
            options.timeout,
        )?;
        interaction_steps.extend(transport_steps);

        Ok(NativeSmokeReport {
            binary_path: binary_path.display().to_string(),
            pid,
            window_title,
            menu_labels,
            menu_contract,
            accessible_name_count: accessible_names.len(),
            accessibility_contract,
            interaction_steps,
            interaction_contract,
            duration_ms: started_at.elapsed().as_millis(),
            closed: true,
        })
    })();

    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }

    let _ = fs::remove_file(&config_path);
    let _ = fs::remove_dir_all(&temp_root);

    result
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", native_smoke_usage());
        return;
    }
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("syncplay-gui-native-smoke failed: {error}");
            std::process::exit(2);
        }
    };

    match run_native_smoke(&options) {
        Ok(report) => {
            print!("{}", report.render(options.format));
        }
        Err(error) => {
            print!("{}", render_error(&error, options.format));
            std::process::exit(1);
        }
    }
}
