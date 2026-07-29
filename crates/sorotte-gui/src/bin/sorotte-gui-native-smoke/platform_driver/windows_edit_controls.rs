use std::{thread, time::Duration};

use super::{PlatformNativeGuiDriver, PlatformWindowHandle};

fn permits_visible_label_fallback(identity: &str) -> bool {
    !identity.starts_with("settings.")
}

impl PlatformNativeGuiDriver {
    fn rect_is_nonempty(rect: &windows::Win32::Foundation::RECT) -> bool {
        rect.right > rect.left && rect.bottom > rect.top
    }

    fn rect_vertical_gap(
        first: &windows::Win32::Foundation::RECT,
        second: &windows::Win32::Foundation::RECT,
    ) -> i32 {
        if first.bottom < second.top {
            second.top - first.bottom
        } else if second.bottom < first.top {
            first.top - second.bottom
        } else {
            0
        }
    }

    fn rect_horizontal_gap(
        first: &windows::Win32::Foundation::RECT,
        second: &windows::Win32::Foundation::RECT,
    ) -> i32 {
        if first.right < second.left {
            second.left - first.right
        } else if second.right < first.left {
            first.left - second.right
        } else {
            0
        }
    }

    fn nearest_editable_element_to_named_label(
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        name: &str,
        edit_elements: &[windows::Win32::UI::Accessibility::IUIAutomationElement],
    ) -> Result<Option<windows::Win32::UI::Accessibility::IUIAutomationElement>, String> {
        use windows::Win32::UI::Accessibility::UIA_EditControlTypeId;

        let elements = Self::collect_subtree_elements(automation, root)?;
        let length = Self::automation_element_count(&elements)?;
        let mut label_rects = Vec::new();
        for index in 0..length {
            let Some(element) = Self::automation_element_at(&elements, index) else {
                continue;
            };
            let element_name = Self::automation_element_name(&element).unwrap_or_default();
            if element_name != name {
                continue;
            }
            if Self::automation_element_control_type(&element) == Some(UIA_EditControlTypeId) {
                continue;
            }
            if Self::automation_element_is_offscreen(&element) {
                continue;
            }
            let Some(rect) = Self::automation_element_bounding_rect(&element) else {
                continue;
            };
            if Self::rect_is_nonempty(&rect) {
                label_rects.push(rect);
            }
        }

        let mut best_match = None;
        for edit_element in edit_elements {
            let Some(edit_rect) = Self::automation_element_bounding_rect(edit_element) else {
                continue;
            };
            if !Self::rect_is_nonempty(&edit_rect) {
                continue;
            }
            for label_rect in &label_rects {
                let edit_center_y = (edit_rect.top + edit_rect.bottom) / 2;
                let label_center_y = (label_rect.top + label_rect.bottom) / 2;
                let vertical_gap = Self::rect_vertical_gap(label_rect, &edit_rect);
                let horizontal_gap = Self::rect_horizontal_gap(label_rect, &edit_rect);
                let direction_penalty = if edit_rect.left >= label_rect.left - 8
                    && edit_rect.top >= label_rect.top - 120
                {
                    0
                } else {
                    10_000
                };
                let score = i64::from(direction_penalty)
                    + i64::from(vertical_gap) * 100
                    + i64::from((edit_center_y - label_center_y).abs())
                    + i64::from(horizontal_gap);
                if best_match
                    .as_ref()
                    .is_none_or(|(best_score, _)| score < *best_score)
                {
                    best_match = Some((score, edit_element.clone()));
                }
            }
        }
        Ok(best_match.map(|(_, element)| element))
    }

    fn collect_editable_elements(
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<Vec<windows::Win32::UI::Accessibility::IUIAutomationElement>, String> {
        use windows::Win32::UI::Accessibility::{IUIAutomationElement, UIA_EditControlTypeId};

        let elements = Self::collect_subtree_elements(automation, root)?;
        let length = Self::automation_element_count(&elements)?;

        let mut edit_elements: Vec<IUIAutomationElement> = Vec::new();
        for index in 0..length {
            let Some(element) = Self::automation_element_at(&elements, index) else {
                continue;
            };
            let Some(current_control_type) = Self::automation_element_control_type(&element) else {
                continue;
            };
            if current_control_type != UIA_EditControlTypeId {
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

            let read_only = Self::automation_value_pattern(&element)
                .as_ref()
                .is_some_and(Self::automation_value_pattern_is_read_only);
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
        Self::focus_window_element(window, element, "edit field for keyboard entry")?;
        thread::sleep(Duration::from_millis(120));
        Self::send_select_all_backspace_and_type(value)
            .map_err(|error| format!("failed keyboard fallback text entry: {error}"))?;
        thread::sleep(Duration::from_millis(120));
        if submit {
            Self::send_enter_key()
                .map_err(|error| format!("failed to submit edit entry with Enter key: {error}"))?;
            thread::sleep(Duration::from_millis(120));
        }
        Ok(())
    }

    pub(super) fn editable_text_input_count(window: PlatformWindowHandle) -> Result<usize, String> {
        use windows::Win32::UI::Accessibility::UIA_EditControlTypeId;

        Self::with_ui_automation(
            window,
            "UI Automation editable text count",
            |automation, root| {
                let elements = Self::collect_subtree_elements(automation, root)?;
                let length = Self::automation_element_count(&elements)?;

                let mut count = 0usize;
                for index in 0..length {
                    let Some(element) = Self::automation_element_at(&elements, index) else {
                        continue;
                    };
                    let Some(current_control_type) =
                        Self::automation_element_control_type(&element)
                    else {
                        continue;
                    };
                    if current_control_type != UIA_EditControlTypeId {
                        continue;
                    }
                    let is_enabled = Self::automation_element_is_enabled(&element);
                    if is_enabled {
                        let is_offscreen = Self::automation_element_is_offscreen(&element);
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
        let pattern = Self::automation_value_pattern_required(element)?;
        Self::automation_value(&pattern)
    }

    pub(super) fn get_named_edit_value(
        window: PlatformWindowHandle,
        name: &str,
    ) -> Result<String, String> {
        Self::with_ui_automation(
            window,
            "UI Automation named edit value lookup",
            |automation, root| {
                let edit_elements = Self::collect_editable_elements(automation, root)?;
                let mut available_names = Vec::new();
                for element in &edit_elements {
                    let element_name = Self::automation_element_name(element).unwrap_or_default();
                    if !element_name.is_empty() {
                        available_names.push(element_name.clone());
                    }
                    let automation_id = Self::automation_element_automation_id(element);
                    if element_name == name || automation_id == name {
                        return Self::read_edit_element_value(element).map_err(|error| {
                            format!("failed to read edit field named {name:?}: {error}")
                        });
                    }
                }
                if permits_visible_label_fallback(name)
                    && let Some(element) = Self::nearest_editable_element_to_named_label(
                        automation,
                        root,
                        name,
                        &edit_elements,
                    )?
                {
                    return Self::read_edit_element_value(&element).map_err(|error| {
                        format!("failed to read edit field nearest label {name:?}: {error}")
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
    pub(super) fn set_named_edit_value(
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
                let mut available_names = Vec::new();
                for element in &edit_elements {
                    let element_name = Self::automation_element_name(element).unwrap_or_default();
                    if !element_name.is_empty() {
                        available_names.push(element_name.clone());
                    }
                    let automation_id = Self::automation_element_automation_id(element);
                    if element_name == name || automation_id == name {
                        return Self::set_edit_element_value(window, element, value, submit)
                            .map_err(|error| {
                                format!("failed to write edit field named {name:?}: {error}")
                            });
                    }
                }
                if permits_visible_label_fallback(name)
                    && let Some(element) = Self::nearest_editable_element_to_named_label(
                        automation,
                        root,
                        name,
                        &edit_elements,
                    )?
                {
                    return Self::set_edit_element_value(window, &element, value, submit).map_err(
                        |error| {
                            format!("failed to write edit field nearest label {name:?}: {error}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_setting_ids_never_fall_back_to_visible_labels() {
        assert!(!permits_visible_label_fallback("settings.connection.host"));
        assert!(permits_visible_label_fallback("Room"));
    }
}
