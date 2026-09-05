use std::{cell::Cell, path::Path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum NativeInputMode {
    #[default]
    StrictPhysical,
    UiaOnly,
}

impl NativeInputMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::StrictPhysical => "strict-physical",
            Self::UiaOnly => "uia-only",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeControlKind {
    Any,
    Button,
}

impl NativeControlKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Any => "control",
            Self::Button => "button",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NativeAccessibilityNode {
    pub(super) name: String,
    pub(super) automation_id: String,
    pub(super) control_type: i32,
    pub(super) enabled: bool,
    pub(super) focused: bool,
    pub(super) offscreen: bool,
    pub(super) bounds: Option<[i32; 4]>,
}

pub(super) trait NativeGuiDriver {
    type WindowHandle: Copy;

    fn native_window_dpi(&self, _window: Self::WindowHandle) -> Result<u32, String> {
        Err("native window DPI measurement is unavailable on this driver".to_owned())
    }

    fn find_main_window(&self, pid: u32) -> Result<Option<Self::WindowHandle>, String>;
    fn prepare_window_for_smoke(&self, window: Self::WindowHandle) -> Result<(), String>;
    fn prepare_window_for_dimensions(
        &self,
        window: Self::WindowHandle,
        width: i32,
        height: i32,
    ) -> Result<(), String>;
    fn press_escape(&self, window: Self::WindowHandle) -> Result<(), String>;
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
    fn scroll_named_content_page(
        &self,
        window: Self::WindowHandle,
        anchor: &NativeAccessibilityNode,
        wheel_delta: i32,
    ) -> Result<(), String> {
        if wheel_delta < 0 {
            self.scroll_named_control_down(window, &anchor.name, NativeControlKind::Any)
        } else {
            self.scroll_named_control_up(window, &anchor.name, NativeControlKind::Any)
        }
    }
    fn window_title(&self, window: Self::WindowHandle) -> Result<String, String>;
    fn accessible_names(&self, window: Self::WindowHandle) -> Result<Vec<String>, String>;
    fn accessibility_nodes(
        &self,
        window: Self::WindowHandle,
    ) -> Result<Vec<NativeAccessibilityNode>, String>;
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
    fn get_named_edit_value(
        &self,
        window: Self::WindowHandle,
        name: &str,
    ) -> Result<String, String>;
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
    fn click_named_control(
        &self,
        window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String>;
    fn activate_named_control_by_keyboard(
        &self,
        _window: Self::WindowHandle,
        name: &str,
        control_kind: NativeControlKind,
    ) -> Result<(), String> {
        Err(format!(
            "focused keyboard activation is unavailable for {} named {name:?}",
            control_kind.label()
        ))
    }
    fn capture_window_png(
        &self,
        window: Self::WindowHandle,
        output_path: &Path,
    ) -> Result<(), String>;
    fn close_window(&self, window: Self::WindowHandle) -> Result<(), String>;
}

#[cfg(target_os = "windows")]
type PlatformWindowHandle = windows_sys::Win32::Foundation::HWND;

#[cfg(not(target_os = "windows"))]
type PlatformWindowHandle = ();

#[derive(Default)]
pub(super) struct PlatformNativeGuiDriver {
    #[cfg(any(target_os = "windows", test))]
    input_mode: NativeInputMode,
    desktop_input_attempts: Cell<usize>,
}

impl PlatformNativeGuiDriver {
    pub(super) fn new(input_mode: NativeInputMode) -> Self {
        #[cfg(all(not(target_os = "windows"), not(test)))]
        let _ = input_mode;

        Self {
            #[cfg(any(target_os = "windows", test))]
            input_mode,
            desktop_input_attempts: Cell::new(0),
        }
    }

    pub(super) fn desktop_input_attempt_count(&self) -> usize {
        self.desktop_input_attempts.get()
    }

    #[cfg(any(target_os = "windows", test))]
    pub(super) fn begin_desktop_input(&self) -> Result<(), String> {
        self.desktop_input_attempts
            .set(self.desktop_input_attempts.get().saturating_add(1));
        if self.input_mode == NativeInputMode::UiaOnly {
            return Err(
                "desktop-wide Win32 input is disabled by --input-mode uia-only (SendInput and cursor movement)"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
#[path = "platform_driver/non_windows_impl.rs"]
mod non_windows_impl;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_automation.rs"]
mod windows_automation;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_control_names.rs"]
mod windows_control_names;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_control_actions.rs"]
mod windows_control_actions;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_control_queries.rs"]
mod windows_control_queries;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_edit_controls.rs"]
mod windows_edit_controls;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_input.rs"]
mod windows_input;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_capture.rs"]
mod windows_capture;

#[cfg(any(target_os = "windows", test))]
#[path = "platform_driver/png.rs"]
mod png;

#[cfg(target_os = "windows")]
#[path = "platform_driver/windows_impl.rs"]
mod windows_impl;

#[cfg(test)]
mod input_policy_tests {
    use super::*;

    #[test]
    fn strict_physical_mode_allows_and_counts_desktop_input() {
        let driver = PlatformNativeGuiDriver::new(NativeInputMode::StrictPhysical);
        driver.begin_desktop_input().unwrap();
        assert_eq!(driver.desktop_input_attempt_count(), 1);
    }

    #[test]
    fn uia_only_mode_blocks_and_counts_desktop_input_before_dispatch() {
        let driver = PlatformNativeGuiDriver::new(NativeInputMode::UiaOnly);
        let error = driver.begin_desktop_input().unwrap_err();
        assert!(error.contains("Win32 input is disabled"));
        assert_eq!(driver.desktop_input_attempt_count(), 1);
    }
}
