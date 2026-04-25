use super::windows_control_names::matches_control_name;
use super::{NativeControlKind, PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
    pub(super) fn count_named_controls(
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
                    if !matches_control_name(name, &current_name) {
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

    pub(super) fn count_named_controls_with_enabled_state(
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
                    if !matches_control_name(name, &current_name) {
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
}
