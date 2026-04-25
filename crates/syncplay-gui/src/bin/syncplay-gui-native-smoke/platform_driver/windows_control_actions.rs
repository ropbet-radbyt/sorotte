use std::{thread, time::Duration};

use super::super::{
    MAIN_WINDOW_CONTROLS_CONTAINER_NAME, MAIN_WINDOW_LOCAL_READY_BUTTON_AUTOMATION_ID,
    MAIN_WINDOW_ROOM_BROWSER_NAME, bool_label,
};
use super::windows_control_names::{is_local_ready_button_request, matches_control_name};
use super::{NativeControlKind, PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
    pub(super) fn invoke_named_control_internal(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        prefer_last: bool,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
            IUIAutomationLegacyIAccessiblePattern, IUIAutomationSelectionItemPattern,
            IUIAutomationTogglePattern, UIA_ExpandCollapsePatternId, UIA_InvokePatternId,
            UIA_LegacyIAccessiblePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        Self::with_ui_automation(window, "UI Automation interaction", |automation, root| {
            let elements = Self::collect_subtree_elements(automation, root)?;
            let length = unsafe {
                elements.Length().map_err(|error| {
                    format!("failed to read UI Automation element count: {error}")
                })?
            };
            let controls_rect = if control_kind == NativeControlKind::Button
                && is_local_ready_button_request(name)
            {
                let mut rect = None;
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
                    if current_name != MAIN_WINDOW_CONTROLS_CONTAINER_NAME {
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
                    rect = unsafe { element.CurrentBoundingRectangle().ok() }
                        .map(|rect| (rect.left, rect.top, rect.right, rect.bottom));
                    if rect.is_some() {
                        break;
                    }
                }
                rect
            } else {
                None
            };
            let mut preferred_candidates = Vec::new();
            let mut fallback_candidates = Vec::new();
            let mut matching_states = Vec::new();
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
                let is_offscreen = unsafe {
                    match element.CurrentIsOffscreen() {
                        Ok(offscreen) => offscreen.as_bool(),
                        Err(_) => false,
                    }
                };
                matching_states.push(format!(
                    "enabled={}, offscreen={}",
                    bool_label(is_enabled),
                    bool_label(is_offscreen)
                ));
                if !is_enabled {
                    continue;
                }
                if is_offscreen {
                    continue;
                }

                let automation_id = unsafe {
                    element
                        .CurrentAutomationId()
                        .map(|value| value.to_string().trim().to_owned())
                        .unwrap_or_default()
                };
                let rect = unsafe { element.CurrentBoundingRectangle().ok() };
                if control_kind == NativeControlKind::Button
                    && is_local_ready_button_request(name)
                    && (automation_id == MAIN_WINDOW_LOCAL_READY_BUTTON_AUTOMATION_ID
                        || if let (
                            Some((controls_left, controls_top, controls_right, controls_bottom)),
                            Some(rect),
                        ) = (controls_rect, rect.as_ref())
                        {
                            let center_x = (rect.left + rect.right) / 2;
                            let center_y = (rect.top + rect.bottom) / 2;
                            center_x >= controls_left
                                && center_x <= controls_right
                                && center_y >= controls_top
                                && center_y <= controls_bottom
                        } else {
                            false
                        })
                {
                    preferred_candidates.push(element);
                } else {
                    fallback_candidates.push(element);
                }
            }

            let mut candidates = if preferred_candidates.is_empty() {
                fallback_candidates
            } else {
                preferred_candidates
            };

            if candidates.is_empty() {
                let matching_state_summary = if matching_states.is_empty() {
                    "none".to_owned()
                } else {
                    matching_states.join(", ")
                };
                return Err(format!(
                    "did not find an enabled {} named {name:?} in the accessibility tree; matching states: {}",
                    control_kind.label(),
                    matching_state_summary,
                ));
            }

            if control_kind == NativeControlKind::Button && is_local_ready_button_request(name) {
                candidates.sort_by_key(|element| unsafe {
                    element
                        .CurrentBoundingRectangle()
                        .map(|rect| rect.top)
                        .unwrap_or(i32::MIN)
                });
                candidates.reverse();
            }

            if prefer_last {
                candidates.reverse();
            }

            let mut invoke_errors = Vec::new();
            for candidate in candidates {
                let mut candidate_errors = Vec::new();

                if control_kind == NativeControlKind::Any && name == MAIN_WINDOW_ROOM_BROWSER_NAME {
                    let focus_result = (|| -> Result<(), String> {
                        unsafe {
                            SetForegroundWindow(window);
                            candidate
                                .SetFocus()
                                .map_err(|error| format!("focus failed: {error}"))?;
                        }
                        thread::sleep(Duration::from_millis(120));
                        Ok(())
                    })();
                    if focus_result.is_ok() {
                        return Ok(());
                    }
                    candidate_errors.push(focus_result.err().unwrap_or_default());
                }

                if control_kind == NativeControlKind::Button && is_local_ready_button_request(name)
                {
                    let click_result = Self::click_element_center(window, &candidate, name);
                    if click_result.is_ok() {
                        return Ok(());
                    }
                    candidate_errors.push(click_result.err().unwrap_or_default());
                }

                let invoke_result = (|| -> Result<(), String> {
                    let invoke_pattern: IUIAutomationInvokePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_InvokePatternId)
                            .map_err(|error| format!("invoke pattern unavailable: {error}"))?
                    };
                    unsafe { invoke_pattern.Invoke() }
                        .map_err(|error| format!("invoke pattern action failed: {error}"))
                })();
                if invoke_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(invoke_result.err().unwrap_or_default());

                let legacy_default_result = (|| -> Result<(), String> {
                    let legacy_pattern: IUIAutomationLegacyIAccessiblePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId)
                            .map_err(|error| {
                                format!("legacy accessible pattern unavailable: {error}")
                            })?
                    };
                    unsafe { legacy_pattern.DoDefaultAction() }
                        .map_err(|error| format!("legacy default action failed: {error}"))
                })();
                if legacy_default_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(legacy_default_result.err().unwrap_or_default());

                let selection_result = (|| -> Result<(), String> {
                    let selection_pattern: IUIAutomationSelectionItemPattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_SelectionItemPatternId)
                            .map_err(|error| {
                                format!("selection-item pattern unavailable: {error}")
                            })?
                    };
                    unsafe { selection_pattern.Select() }
                        .map_err(|error| format!("selection-item action failed: {error}"))
                })();
                if selection_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(selection_result.err().unwrap_or_default());

                let toggle_result = (|| -> Result<(), String> {
                    let toggle_pattern: IUIAutomationTogglePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_TogglePatternId)
                            .map_err(|error| format!("toggle pattern unavailable: {error}"))?
                    };
                    unsafe { toggle_pattern.Toggle() }
                        .map_err(|error| format!("toggle action failed: {error}"))
                })();
                if toggle_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(toggle_result.err().unwrap_or_default());

                let expand_result = (|| -> Result<(), String> {
                    let expand_pattern: IUIAutomationExpandCollapsePattern = unsafe {
                        candidate
                            .GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
                            .map_err(|error| {
                                format!("expand-collapse pattern unavailable: {error}")
                            })?
                    };
                    unsafe { expand_pattern.Expand() }
                        .map_err(|error| format!("expand action failed: {error}"))
                })();
                if expand_result.is_ok() {
                    return Ok(());
                }
                candidate_errors.push(expand_result.err().unwrap_or_default());

                invoke_errors.push(candidate_errors.join("; "));
            }

            Err(format!(
                "failed to invoke {} named {name:?}: {}",
                control_kind.label(),
                invoke_errors.join("; ")
            ))
        })
    }

    pub(super) fn scroll_named_control_internal(
        window: PlatformWindowHandle,
        name: &str,
        control_kind: NativeControlKind,
        wheel_delta: i32,
    ) -> Result<(), String> {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, SetCursorPos, SetForegroundWindow,
        };

        Self::with_ui_automation(
            window,
            "UI Automation scroll interaction",
            |automation, root| {
                let elements = Self::collect_subtree_elements(automation, root)?;
                let length = unsafe {
                    elements.Length().map_err(|error| {
                        format!("failed to read UI Automation element count: {error}")
                    })?
                };

                let mut matching_states = Vec::new();
                let mut target_center = None;
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
                    let is_offscreen = unsafe {
                        match element.CurrentIsOffscreen() {
                            Ok(offscreen) => offscreen.as_bool(),
                            Err(_) => false,
                        }
                    };
                    matching_states.push(format!(
                        "enabled={}, offscreen={}",
                        bool_label(is_enabled),
                        bool_label(is_offscreen)
                    ));
                    if !is_enabled || is_offscreen {
                        continue;
                    }

                    let rect = unsafe {
                        element.CurrentBoundingRectangle().map_err(|error| {
                            format!("failed to read UI Automation bounding rectangle: {error}")
                        })?
                    };
                    if rect.right <= rect.left || rect.bottom <= rect.top {
                        continue;
                    }

                    target_center =
                        Some(((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2));
                    break;
                }

                let Some((center_x, center_y)) = target_center else {
                    let matching_state_summary = if matching_states.is_empty() {
                        "none".to_owned()
                    } else {
                        matching_states.join(", ")
                    };
                    return Err(format!(
                        "did not find a visible {} named {name:?} for scrolling; matching states: {}",
                        control_kind.label(),
                        matching_state_summary
                    ));
                };

                let mut original_cursor = POINT { x: 0, y: 0 };
                unsafe {
                    SetForegroundWindow(window);
                    let _ = GetCursorPos(&mut original_cursor);
                    let set_cursor_result = SetCursorPos(center_x, center_y);
                    if set_cursor_result == 0 {
                        return Err(format!(
                            "failed to move cursor to {name:?} center at ({center_x}, {center_y})"
                        ));
                    }
                }
                thread::sleep(Duration::from_millis(80));
                let wheel_result = Self::send_mouse_wheel(wheel_delta)
                    .map_err(|error| format!("failed to send mouse-wheel input: {error}"));
                unsafe {
                    let _ = SetCursorPos(original_cursor.x, original_cursor.y);
                }
                wheel_result?;
                thread::sleep(Duration::from_millis(120));
                Ok(())
            },
        )
    }
}
