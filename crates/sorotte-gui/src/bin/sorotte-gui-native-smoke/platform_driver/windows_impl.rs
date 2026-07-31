use std::{thread, time::Duration};

use super::super::{SMOKE_WINDOW_HEIGHT, SMOKE_WINDOW_WIDTH, SMOKE_WINDOW_X, SMOKE_WINDOW_Y};
use super::{
    NativeAccessibilityNode, NativeControlKind, NativeGuiDriver, PlatformNativeGuiDriver,
    PlatformWindowHandle,
};

#[cfg(target_os = "windows")]
impl PlatformNativeGuiDriver {
    fn retry_transient_automation_read<T>(
        mut read: impl FnMut() -> Result<T, String>,
    ) -> Result<T, String> {
        const MAX_ATTEMPTS: usize = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            match read() {
                Ok(value) => return Ok(value),
                Err(error) if error.contains("0x80040201") && attempt < MAX_ATTEMPTS => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded UI Automation retry loop always returns on its final attempt")
    }

    fn window_text_for_handle(window: PlatformWindowHandle) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

        // SAFETY: `window` is an HWND discovered by the native smoke driver; invalid or closed
        // windows are handled by returning an empty title.
        let text_length = unsafe { GetWindowTextLengthW(window) };
        if text_length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; text_length as usize + 1];
        // SAFETY: `buffer` has space for the title plus trailing NUL and is valid for writes for
        // the duration of the call.
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

    // SAFETY: `lparam` is passed by `find_main_window` as a valid `FindWindowContext` pointer for
    // the duration of the synchronous `EnumWindows` callback.
    let context = unsafe { &mut *(lparam as *mut FindWindowContext) };
    let mut window_pid = 0u32;
    // SAFETY: `window` is supplied by EnumWindows and `window_pid` is a valid out-parameter.
    unsafe {
        GetWindowThreadProcessId(window, &mut window_pid);
    }
    if window_pid != context.pid {
        return 1;
    }
    // SAFETY: `window` is supplied by EnumWindows; a non-visible or invalid window is skipped.
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
        // SAFETY: The callback and context pointer remain valid for the duration of the
        // synchronous EnumWindows call.
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
        Self::prepare_visible_window_bounds(
            window,
            SMOKE_WINDOW_X,
            SMOKE_WINDOW_Y,
            SMOKE_WINDOW_WIDTH,
            SMOKE_WINDOW_HEIGHT,
        )
    }

    fn prepare_window_for_dimensions(
        &self,
        window: Self::WindowHandle,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        Self::prepare_visible_window_bounds(window, SMOKE_WINDOW_X, SMOKE_WINDOW_Y, width, height)
    }

    fn press_escape(&self, window: Self::WindowHandle) -> Result<(), String> {
        Self::ensure_window_foreground(window, "native smoke window before Escape")?;
        thread::sleep(Duration::from_millis(80));
        self.send_escape_key()
            .map_err(|error| format!("failed to send Escape to native smoke window: {error}"))?;
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn scroll_active_view_page_down(&self, window: Self::WindowHandle) -> Result<(), String> {
        Self::ensure_window_foreground(window, "native smoke window before PageDown")?;
        thread::sleep(Duration::from_millis(80));
        self.send_page_down_key()
            .map_err(|error| format!("failed to send PageDown to native smoke window: {error}"))?;
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    fn scroll_active_view_page_up(&self, window: Self::WindowHandle) -> Result<(), String> {
        Self::ensure_window_foreground(window, "native smoke window before PageUp")?;
        thread::sleep(Duration::from_millis(80));
        self.send_page_up_key()
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
        self.scroll_named_control_internal(window, name, control_kind, -120)
    }

    fn scroll_named_control_up(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        self.scroll_named_control_internal(window, name, control_kind, 120)
    }

    fn window_title(&self, window: Self::WindowHandle) -> Result<String, String> {
        Ok(Self::window_text_for_handle(window))
    }

    fn accessible_names(&self, window: Self::WindowHandle) -> Result<Vec<String>, String> {
        Self::retry_transient_automation_read(|| Self::collect_accessible_names(window))
    }

    fn accessibility_nodes(
        &self,
        window: Self::WindowHandle,
    ) -> Result<Vec<NativeAccessibilityNode>, String> {
        Self::retry_transient_automation_read(|| Self::collect_accessibility_nodes(window))
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

    fn get_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
    ) -> Result<String, String> {
        Self::get_named_edit_value(window, name)
    }

    fn set_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
        value: &str,
        submit: bool,
    ) -> Result<(), String> {
        PlatformNativeGuiDriver::set_named_edit_value(self, window, name, value, submit)
    }

    fn invoke_named_control(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        self.invoke_named_control_internal(window, name, control_kind, false, false)
    }

    fn click_named_control(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        self.invoke_named_control_internal(window, name, control_kind, false, true)
    }

    fn activate_named_control_by_keyboard(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        self.activate_named_control_by_keyboard_internal(window, name, control_kind)
    }

    fn capture_window_png(
        &self,
        window: Self::WindowHandle,
        output_path: &std::path::Path,
    ) -> Result<(), String> {
        Self::capture_window_png_internal(window, output_path)
    }

    fn close_window(&self, window: Self::WindowHandle) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CLOSE};

        // SAFETY: `window` is the GUI HWND under test. WM_CLOSE is sent synchronously as part of
        // smoke-test cleanup.
        unsafe {
            SendMessageW(window, WM_CLOSE, 0, 0);
        }
        Ok(())
    }
}
