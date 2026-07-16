use super::windows_control_names::matches_control_identity;
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
                let length = Self::automation_element_count(&elements)?;

                let mut count = 0usize;
                for index in 0..length {
                    let Some(element) = Self::automation_element_at(&elements, index) else {
                        continue;
                    };
                    let Some(current_name) = Self::automation_element_name(&element) else {
                        continue;
                    };
                    let automation_id = Self::automation_element_automation_id(&element);
                    if !matches_control_identity(name, &current_name, &automation_id) {
                        continue;
                    }

                    let Some(current_control_type) =
                        Self::automation_element_control_type(&element)
                    else {
                        continue;
                    };
                    if !control_kind.matches_control_type(current_control_type) {
                        continue;
                    }

                    let is_enabled = Self::automation_element_is_enabled(&element);
                    if !is_enabled {
                        continue;
                    }
                    let is_offscreen = Self::automation_element_is_offscreen(&element);
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
                let length = Self::automation_element_count(&elements)?;

                let mut count = 0usize;
                for index in 0..length {
                    let Some(element) = Self::automation_element_at(&elements, index) else {
                        continue;
                    };
                    let Some(current_name) = Self::automation_element_name(&element) else {
                        continue;
                    };
                    let automation_id = Self::automation_element_automation_id(&element);
                    if !matches_control_identity(name, &current_name, &automation_id) {
                        continue;
                    }

                    let Some(current_control_type) =
                        Self::automation_element_control_type(&element)
                    else {
                        continue;
                    };
                    if !control_kind.matches_control_type(current_control_type) {
                        continue;
                    }

                    let is_enabled = Self::automation_element_is_enabled(&element);
                    if is_enabled != expected_enabled {
                        continue;
                    }
                    let is_offscreen = Self::automation_element_is_offscreen(&element);
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
