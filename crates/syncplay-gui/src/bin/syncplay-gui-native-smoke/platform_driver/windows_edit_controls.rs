use std::{thread, time::Duration};

use super::{PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
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
        let value_pattern = Self::automation_value_pattern(element);
        if let Some(pattern) = value_pattern.as_ref()
            && Self::set_automation_value(pattern, value).is_ok()
        {
            thread::sleep(Duration::from_millis(120));
            let actual = Self::automation_value(pattern).unwrap_or_default();
            if actual == value {
                if !submit {
                    return Ok(());
                }
                Self::focus_window_element(window, element, "edit field for submit key entry")?;
                thread::sleep(Duration::from_millis(120));
                Self::send_enter_key().map_err(|error| {
                    format!("failed to submit edit entry with Enter key: {error}")
                })?;
                thread::sleep(Duration::from_millis(120));
                return Ok(());
            }
        }

        Self::focus_window_element(window, element, "edit field for keyboard entry")?;
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
        let actual = Self::automation_value(&verification_pattern).unwrap_or_default();
        if actual == value {
            Ok(())
        } else {
            Err(format!(
                "keyboard fallback set edit field to {actual:?}, expected {value:?}"
            ))
        }
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

    pub(super) fn get_edit_value_by_index(
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

    pub(super) fn get_named_edit_value(
        window: PlatformWindowHandle,
        name: &str,
    ) -> Result<String, String> {
        Self::with_ui_automation(
            window,
            "UI Automation named edit value lookup",
            |automation, root| {
                let edit_elements = Self::collect_editable_elements(automation, root)?;
                let mut fallback_last_edit = None;
                let mut available_names = Vec::new();
                for element in edit_elements {
                    fallback_last_edit = Some(element.clone());
                    let element_name = Self::automation_element_name(&element).unwrap_or_default();
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
    pub(super) fn set_edit_value_by_index(
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
                let mut fallback_last_edit = None;
                let mut available_names = Vec::new();
                for element in edit_elements {
                    fallback_last_edit = Some(element.clone());
                    let element_name = Self::automation_element_name(&element).unwrap_or_default();
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
