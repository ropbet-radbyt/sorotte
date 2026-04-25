use std::{thread, time::Duration};

use super::{PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
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

    pub(super) fn editable_text_input_count(window: PlatformWindowHandle) -> Result<usize, String> {
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
