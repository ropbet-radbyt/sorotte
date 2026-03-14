#[cfg(target_os = "windows")]
use std::{thread, time::Duration};

#[cfg(target_os = "windows")]
use super::{
    MAIN_WINDOW_CONTROLS_CONTAINER_NAME, MAIN_WINDOW_LOCAL_READY_BUTTON_AUTOMATION_ID,
    MAIN_WINDOW_LOCAL_READY_BUTTON_NAME, MAIN_WINDOW_ROOM_BROWSER_NAME, SMOKE_WINDOW_HEIGHT,
    SMOKE_WINDOW_WIDTH, SMOKE_WINDOW_X, SMOKE_WINDOW_Y, bool_label,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeControlKind {
    Any,
    Button,
    MenuItem,
}

impl NativeControlKind {
    pub(super) fn label(self) -> &'static str {
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

pub(super) trait NativeGuiDriver {
    type WindowHandle: Copy;

    fn find_main_window(&self, pid: u32) -> Result<Option<Self::WindowHandle>, String>;
    fn prepare_window_for_smoke(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn scroll_active_view_page_down(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn scroll_active_view_page_up(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn scroll_named_control_down(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String>;
    fn scroll_named_control_up(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String>;
    fn window_title(&self, window: Self::WindowHandle) -> Result<String, String>;
    fn accessible_names(&self, window: Self::WindowHandle) -> Result<Vec<String>, String>;
    fn top_level_menu_labels(&self, window: Self::WindowHandle) -> Result<Vec<String>, String>;
    fn count_named_controls(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<usize, String>;
    fn count_named_controls_with_enabled_state(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        enabled: bool,
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
pub(super) struct PlatformNativeGuiDriver;

#[cfg(not(target_os = "windows"))]
#[derive(Default)]
pub(super) struct PlatformNativeGuiDriver;

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

    fn mouse_input(
        flags: windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
        data: u32,
    ) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
        };

        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn mouse_input_for_wheel(delta: i32) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOUSEEVENTF_WHEEL;

        Self::mouse_input(MOUSEEVENTF_WHEEL, delta as u32)
    }

    fn send_mouse_wheel(delta: i32) -> Result<(), String> {
        let mut inputs = [Self::mouse_input_for_wheel(delta)];
        Self::send_keyboard_inputs(&mut inputs)
    }

    fn click_element_center(
        window: PlatformWindowHandle,
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        name: &str,
    ) -> Result<(), String> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageW, SetForegroundWindow, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };

        unsafe extern "system" {
            fn ScreenToClient(hwnd: PlatformWindowHandle, point: *mut POINT) -> i32;
        }

        let rect = unsafe {
            element.CurrentBoundingRectangle().map_err(|error| {
                format!("failed to read UI Automation bounding rectangle for {name:?}: {error}")
            })?
        };
        if rect.right <= rect.left || rect.bottom <= rect.top {
            return Err(format!(
                "UI Automation bounding rectangle for {name:?} was empty"
            ));
        }

        let center_x = (rect.left + rect.right) / 2;
        let center_y = (rect.top + rect.bottom) / 2;
        let mut client_point = POINT {
            x: center_x,
            y: center_y,
        };
        unsafe {
            SetForegroundWindow(window);
            let _ = element.SetFocus();
            if ScreenToClient(window, &mut client_point) == 0 {
                return Err(format!(
                    "failed to convert {name:?} center ({center_x}, {center_y}) to client coordinates"
                ));
            }
        }
        thread::sleep(Duration::from_millis(80));
        unsafe {
            let lparam =
                ((client_point.y as u32) << 16 | (client_point.x as u32 & 0xffff)) as isize;
            SendMessageW(window, WM_MOUSEMOVE, 0, lparam);
            SendMessageW(window, WM_LBUTTONDOWN, 0x0001, lparam);
            SendMessageW(window, WM_LBUTTONUP, 0, lparam);
        }
        thread::sleep(Duration::from_millis(120));
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
        for ch in value.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                if !text_inputs.is_empty() {
                    Self::send_keyboard_inputs(&mut text_inputs)?;
                    text_inputs.clear();
                }
                Self::send_enter_key()?;
                continue;
            }
            let mut utf16_buffer = [0u16; 2];
            for code_unit in ch.encode_utf16(&mut utf16_buffer) {
                text_inputs.push(Self::keyboard_input_for_unicode(*code_unit, 0));
                text_inputs.push(Self::keyboard_input_for_unicode(
                    *code_unit,
                    KEYEVENTF_KEYUP,
                ));
            }
        }
        if text_inputs.is_empty() {
            Ok(())
        } else {
            Self::send_keyboard_inputs(&mut text_inputs)
        }
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

    fn invoke_named_control_internal(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        prefer_last: bool,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
            IUIAutomationLegacyIAccessiblePattern, IUIAutomationSelectionItemPattern,
            IUIAutomationTogglePattern, UIA_ExpandCollapsePatternId, UIA_InvokePatternId,
            UIA_LegacyIAccessiblePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        Self::with_ui_automation(window, "UI Automation interaction", |automation, root| {
            let elements = Self::collect_subtree_elements(automation, root)?;
            let length = unsafe {
                elements.Length().map_err(|error| {
                    format!("failed to read UI Automation element count: {error}")
                })?
            };
            let controls_rect = if control_kind == NativeControlKind::Button
                && name == MAIN_WINDOW_LOCAL_READY_BUTTON_NAME
            {
                let mut rect = None;
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
                    if current_name != MAIN_WINDOW_CONTROLS_CONTAINER_NAME {
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
                    rect = unsafe { element.CurrentBoundingRectangle().ok() }
                        .map(|rect| (rect.left, rect.top, rect.right, rect.bottom));
                    if rect.is_some() {
                        break;
                    }
                }
                rect
            } else {
                None
            };
            let mut preferred_candidates = Vec::new();
            let mut fallback_candidates = Vec::new();
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

                let automation_id = unsafe {
                    element
                        .CurrentAutomationId()
                        .map(|value| value.to_string().trim().to_owned())
                        .unwrap_or_default()
                };
                let rect = unsafe { element.CurrentBoundingRectangle().ok() };
                if control_kind == NativeControlKind::Button
                    && name == MAIN_WINDOW_LOCAL_READY_BUTTON_NAME
                    && (automation_id == MAIN_WINDOW_LOCAL_READY_BUTTON_AUTOMATION_ID
                        || if let (
                            Some((controls_left, controls_top, controls_right, controls_bottom)),
                            Some(rect),
                        ) = (controls_rect, rect.as_ref())
                        {
                            let center_x = (rect.left + rect.right) / 2;
                            let center_y = (rect.top + rect.bottom) / 2;
                            center_x >= controls_left
                                && center_x <= controls_right
                                && center_y >= controls_top
                                && center_y <= controls_bottom
                        } else {
                            false
                        })
                {
                    preferred_candidates.push(element);
                } else {
                    fallback_candidates.push(element);
                }
            }

            let mut candidates = if preferred_candidates.is_empty() {
                fallback_candidates
            } else {
                preferred_candidates
            };

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

            if control_kind == NativeControlKind::Button
                && name == MAIN_WINDOW_LOCAL_READY_BUTTON_NAME
            {
                candidates.sort_by_key(|element| unsafe {
                    element
                        .CurrentBoundingRectangle()
                        .map(|rect| rect.top)
                        .unwrap_or(i32::MIN)
                });
                candidates.reverse();
            }

            if prefer_last {
                candidates.reverse();
            }

            let mut invoke_errors = Vec::new();
            for candidate in candidates {
                let mut candidate_errors = Vec::new();

                if control_kind == NativeControlKind::Any && name == MAIN_WINDOW_ROOM_BROWSER_NAME {
                    let focus_result = (|| -> Result<(), String> {
                        unsafe {
                            SetForegroundWindow(window);
                            candidate
                                .SetFocus()
                                .map_err(|error| format!("focus failed: {error}"))?;
                        }
                        thread::sleep(Duration::from_millis(120));
                        Ok(())
                    })();
                    if focus_result.is_ok() {
                        return Ok(());
                    }
                    candidate_errors.push(focus_result.err().unwrap_or_default());
                }

                if control_kind == NativeControlKind::Button
                    && name == MAIN_WINDOW_LOCAL_READY_BUTTON_NAME
                {
                    let click_result = Self::click_element_center(window, &candidate, name);
                    if click_result.is_ok() {
                        return Ok(());
                    }
                    candidate_errors.push(click_result.err().unwrap_or_default());
                }

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

                let legacy_default_result = (|| -> Result<(), String> {
                    let legacy_pattern: IUIAutomationLegacyIAccessiblePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId)
                            .map_err(|error| {
                                format!("legacy accessible pattern unavailable: {error}")
                            })?
                    };
                    unsafe { legacy_pattern.DoDefaultAction() }
                        .map_err(|error| format!("legacy default action failed: {error}"))
                })();
                if legacy_default_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(legacy_default_result.err().unwrap_or_default());

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

    fn scroll_named_control_internal(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        wheel_delta: i32,
    ) -> Result<(), String> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, SetCursorPos, SetForegroundWindow,
        };

        Self::with_ui_automation(
            window,
            "UI Automation scroll interaction",
            |automation, root| {
                let elements = Self::collect_subtree_elements(automation, root)?;
                let length = unsafe {
                    elements.Length().map_err(|error| {
                        format!("failed to read UI Automation element count: {error}")
                    })?
                };

                let mut matching_states = Vec::new();
                let mut target_center = None;
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
                    if !is_enabled || is_offscreen {
                        continue;
                    }

                    let rect = unsafe {
                        element.CurrentBoundingRectangle().map_err(|error| {
                            format!("failed to read UI Automation bounding rectangle: {error}")
                        })?
                    };
                    if rect.right <= rect.left || rect.bottom <= rect.top {
                        continue;
                    }

                    target_center =
                        Some(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2));
                    break;
                }

                let Some((center_x, center_y)) = target_center else {
                    let matching_state_summary = if matching_states.is_empty() {
                        "none".to_owned()
                    } else {
                        matching_states.join(", ")
                    };
                    return Err(format!(
                        "did not find a visible {} named {name:?} for scrolling; matching states: {}",
                        control_kind.label(),
                        matching_state_summary
                    ));
                };

                let mut original_cursor = POINT { x: 0, y: 0 };
                unsafe {
                    SetForegroundWindow(window);
                    let _ = GetCursorPos(&mut original_cursor);
                    let set_cursor_result = SetCursorPos(center_x, center_y);
                    if set_cursor_result == 0 {
                        return Err(format!(
                            "failed to move cursor to {name:?} center at ({center_x}, {center_y})"
                        ));
                    }
                }
                thread::sleep(Duration::from_millis(80));
                let wheel_result = Self::send_mouse_wheel(wheel_delta)
                    .map_err(|error| format!("failed to send mouse-wheel input: {error}"));
                unsafe {
                    let _ = SetCursorPos(original_cursor.x, original_cursor.y);
                }
                wheel_result?;
                thread::sleep(Duration::from_millis(120));
                Ok(())
            },
        )
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

    fn count_named_controls_with_enabled_state(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        expected_enabled: bool,
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
                    if is_enabled != expected_enabled {
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
            SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
        };

        unsafe {
            SetForegroundWindow(window);
            let result = SetWindowPos(
                window,
                std::ptr::null_mut(),
                SMOKE_WINDOW_X,
                SMOKE_WINDOW_Y,
                SMOKE_WINDOW_WIDTH,
                SMOKE_WINDOW_HEIGHT,
                SWP_NOZORDER,
            );
            if result == 0 {
                return Err("failed to set native smoke window bounds".to_owned());
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

    fn scroll_named_control_down(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        Self::scroll_named_control_internal(window, name, control_kind, -120)
    }

    fn scroll_named_control_up(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        Self::scroll_named_control_internal(window, name, control_kind, 120)
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

    fn count_named_controls_with_enabled_state(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        enabled: bool,
    ) -> Result<usize, String> {
        Self::count_named_controls_with_enabled_state(window, name, control_kind, enabled)
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
        Self::invoke_named_control_internal(window, name, control_kind, false)
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

    fn scroll_named_control_down(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
        _control_kind: NativeControlKind,
    ) -> Result<(), String> {
        Ok(())
    }

    fn scroll_named_control_up(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
        _control_kind: NativeControlKind,
    ) -> Result<(), String> {
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

    fn count_named_controls_with_enabled_state(
        &self,
        _window: Self::WindowHandle,
        _name: &str,
        _control_kind: NativeControlKind,
        _enabled: bool,
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
