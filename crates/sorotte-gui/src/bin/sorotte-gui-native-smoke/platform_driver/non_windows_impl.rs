use super::{NativeControlKind, NativeGuiDriver, PlatformNativeGuiDriver, PlatformWindowHandle};

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
