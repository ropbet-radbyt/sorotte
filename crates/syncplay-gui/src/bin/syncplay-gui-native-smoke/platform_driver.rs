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

#[derive(Default)]
pub(super) struct PlatformNativeGuiDriver;

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
#[path = "platform_driver/windows_impl.rs"]
mod windows_impl;
