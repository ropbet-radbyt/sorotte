use std::{thread, time::Duration};

use super::super::{SMOKE_WINDOW_HEIGHT, SMOKE_WINDOW_WIDTH, SMOKE_WINDOW_X, SMOKE_WINDOW_Y};
use super::{NativeControlKind, NativeGuiDriver, PlatformNativeGuiDriver, PlatformWindowHandle};

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
