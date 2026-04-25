use super::{PlatformNativeGuiDriver, PlatformWindowHandle};

impl PlatformNativeGuiDriver {
    pub(super) fn with_ui_automation<T, F>(
        window: PlatformWindowHandle,
        operation_label: &str,
        run: F,
    ) -> Result<T, String>
    where
        F: FnOnce(
            &windows::Win32::UI::Accessibility::IUIAutomation,
            &windows::Win32::UI::Accessibility::IUIAutomationElement,
        ) -> Result<T, String>,
    {
        use windows::{
            Win32::{
                Foundation::HWND,
                System::Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoUninitialize,
                },
                UI::Accessibility::{CUIAutomation, IUIAutomation},
            },
            core::HRESULT,
        };

        struct ComScope {
            should_uninitialize: bool,
        }

        impl ComScope {
            fn initialize(operation_label: &str) -> Result<Self, String> {
                // SAFETY: Native smoke UI Automation calls run on the current driver thread.
                // COM is initialized for STA use here and balanced by `ComScope::drop`.
                let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
                if hr.is_ok() {
                    return Ok(Self {
                        should_uninitialize: true,
                    });
                }
                let rpc_e_changed_mode = HRESULT(0x8001_0106u32 as i32);
                if hr == rpc_e_changed_mode {
                    return Ok(Self {
                        should_uninitialize: false,
                    });
                }
                Err(format!(
                    "failed to initialize COM for {operation_label}: 0x{:08X}",
                    hr.0 as u32
                ))
            }
        }

        impl Drop for ComScope {
            fn drop(&mut self) {
                if self.should_uninitialize {
                    // SAFETY: `should_uninitialize` is true only when this scope successfully
                    // initialized COM on the same thread.
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }

        let _com = ComScope::initialize(operation_label)?;
        // SAFETY: COM has been initialized for this thread by `ComScope`; the class/context
        // arguments are the standard in-process UI Automation activation path.
        let automation: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("failed to create IUIAutomation instance: {error}"))?
        };
        // SAFETY: `window` is the HWND for the GUI process under test. UI Automation owns the
        // resulting COM interface and errors are converted into driver failures.
        let root = unsafe {
            automation
                .ElementFromHandle(HWND(window))
                .map_err(|error| {
                    format!("failed to map main window handle into UI Automation element: {error}")
                })?
        };
        run(&automation, &root)
    }

    pub(super) fn automation_element_count(
        elements: &windows::Win32::UI::Accessibility::IUIAutomationElementArray,
    ) -> Result<i32, String> {
        // SAFETY: Native smoke tests obtain element arrays from `with_ui_automation`, which
        // initializes COM for the current thread before handing out UI Automation interfaces.
        unsafe {
            elements
                .Length()
                .map_err(|error| format!("failed to read UI Automation element count: {error}"))
        }
    }

    pub(super) fn automation_element_at(
        elements: &windows::Win32::UI::Accessibility::IUIAutomationElementArray,
        index: i32,
    ) -> Option<windows::Win32::UI::Accessibility::IUIAutomationElement> {
        // SAFETY: The element array belongs to the active UI Automation traversal; failed
        // cross-process lookups are expected during smoke tests and are skipped by callers.
        unsafe { elements.GetElement(index).ok() }
    }

    pub(super) fn automation_element_at_required(
        elements: &windows::Win32::UI::Accessibility::IUIAutomationElementArray,
        index: i32,
    ) -> Result<windows::Win32::UI::Accessibility::IUIAutomationElement, String> {
        // SAFETY: The element array belongs to the active UI Automation traversal; HRESULT
        // failures are converted into test-driver errors.
        unsafe {
            elements.GetElement(index).map_err(|error| {
                format!("failed to fetch UI Automation element at index {index}: {error}")
            })
        }
    }

    pub(super) fn automation_element_name(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Option<String> {
        // SAFETY: The interface is owned by the current UI Automation traversal; unavailable
        // properties are represented as missing names for resilient smoke-driver matching.
        unsafe { element.CurrentName().ok() }.map(|value| value.to_string().trim().to_owned())
    }

    pub(super) fn automation_element_name_required(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<String, String> {
        // SAFETY: The interface is owned by the current UI Automation traversal; HRESULT
        // failures are surfaced because discovery output requires a complete name read.
        unsafe {
            element
                .CurrentName()
                .map(|value| value.to_string().trim().to_owned())
                .map_err(|error| format!("failed to read UI Automation element name: {error}"))
        }
    }

    pub(super) fn automation_element_automation_id(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> String {
        // SAFETY: The interface is owned by the current UI Automation traversal; controls may
        // omit automation IDs, so callers treat lookup failure as an empty ID.
        unsafe { element.CurrentAutomationId().ok() }
            .map(|value| value.to_string().trim().to_owned())
            .unwrap_or_default()
    }

    pub(super) fn automation_element_control_type(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Option<windows::Win32::UI::Accessibility::UIA_CONTROLTYPE_ID> {
        // SAFETY: The interface is owned by the current UI Automation traversal; failed property
        // reads are ignored so smoke tests can keep walking partially available trees.
        unsafe { element.CurrentControlType().ok() }
    }

    pub(super) fn automation_element_is_enabled(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> bool {
        // SAFETY: The interface is owned by the current UI Automation traversal; if enabled state
        // cannot be read, treating it as disabled prevents unsafe interaction attempts.
        unsafe {
            element
                .CurrentIsEnabled()
                .map(|enabled| enabled.as_bool())
                .unwrap_or(false)
        }
    }

    pub(super) fn automation_element_is_offscreen(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> bool {
        // SAFETY: The interface is owned by the current UI Automation traversal; lookup failures
        // fall back to "onscreen" to preserve the driver's previous matching behavior.
        unsafe {
            element
                .CurrentIsOffscreen()
                .map(|offscreen| offscreen.as_bool())
                .unwrap_or(false)
        }
    }

    pub(super) fn automation_element_bounding_rect(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Option<windows::Win32::Foundation::RECT> {
        // SAFETY: The interface is owned by the current UI Automation traversal; missing
        // rectangles are treated as absent geometry by callers.
        unsafe { element.CurrentBoundingRectangle().ok() }
    }

    pub(super) fn automation_element_bounding_rect_required(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        context: &str,
    ) -> Result<windows::Win32::Foundation::RECT, String> {
        // SAFETY: The interface is owned by the current UI Automation traversal; HRESULT failures
        // become explicit smoke-driver errors for actions that require geometry.
        unsafe {
            element.CurrentBoundingRectangle().map_err(|error| {
                format!("failed to read UI Automation bounding rectangle for {context}: {error}")
            })
        }
    }

    pub(super) fn automation_value_pattern(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Option<windows::Win32::UI::Accessibility::IUIAutomationValuePattern> {
        use windows::Win32::UI::Accessibility::UIA_ValuePatternId;

        // SAFETY: The element comes from the current UI Automation traversal; absence of the value
        // pattern is a normal control capability check.
        unsafe { element.GetCurrentPatternAs(UIA_ValuePatternId).ok() }
    }

    pub(super) fn automation_value_pattern_required(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<windows::Win32::UI::Accessibility::IUIAutomationValuePattern, String> {
        use windows::Win32::UI::Accessibility::UIA_ValuePatternId;

        // SAFETY: The element comes from the current UI Automation traversal; a missing value
        // pattern is reported because edit-value reads require it.
        unsafe {
            element
                .GetCurrentPatternAs(UIA_ValuePatternId)
                .map_err(|error| format!("value pattern unavailable for edit control: {error}"))
        }
    }

    pub(super) fn automation_value_pattern_is_read_only(
        pattern: &windows::Win32::UI::Accessibility::IUIAutomationValuePattern,
    ) -> bool {
        // SAFETY: The pattern was obtained from the active UI Automation element; unreadable
        // state is treated as read-only to avoid writing to ambiguous controls.
        unsafe {
            pattern
                .CurrentIsReadOnly()
                .map(|flag| flag.as_bool())
                .unwrap_or(true)
        }
    }

    pub(super) fn set_automation_value(
        pattern: &windows::Win32::UI::Accessibility::IUIAutomationValuePattern,
        value: &str,
    ) -> Result<(), String> {
        use windows::core::BSTR;

        let value_bstr = BSTR::from(value);
        // SAFETY: The pattern was obtained from the active UI Automation element; BSTR is owned
        // for the duration of the call and HRESULT failures are returned to the caller.
        unsafe { pattern.SetValue(&value_bstr) }
            .map_err(|error| format!("failed to set UI Automation value: {error}"))
    }

    pub(super) fn automation_value(
        pattern: &windows::Win32::UI::Accessibility::IUIAutomationValuePattern,
    ) -> Result<String, String> {
        // SAFETY: The pattern was obtained from the active UI Automation element; HRESULT
        // failures are returned as smoke-driver errors.
        unsafe { pattern.CurrentValue() }
            .map(|value| value.to_string())
            .map_err(|error| format!("failed to read edit control value: {error}"))
    }

    pub(super) fn focus_window_element(
        window: PlatformWindowHandle,
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
        context: &str,
    ) -> Result<(), String> {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

        // SAFETY: `window` is the GUI process window under test and `element` comes from the
        // active UI Automation traversal. Focus failures are converted into driver errors.
        unsafe {
            SetForegroundWindow(window);
            element
                .SetFocus()
                .map_err(|error| format!("failed to focus {context}: {error}"))
        }
    }

    pub(super) fn invoke_automation_element(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{IUIAutomationInvokePattern, UIA_InvokePatternId};

        // SAFETY: The element comes from the current UI Automation traversal; pattern lookup and
        // invocation failures are converted into smoke-driver errors.
        let invoke_pattern: IUIAutomationInvokePattern = unsafe {
            element
                .GetCurrentPatternAs(UIA_InvokePatternId)
                .map_err(|error| format!("invoke pattern unavailable: {error}"))?
        };
        // SAFETY: The invoke pattern was obtained from the active UI Automation element and is
        // used immediately on the same thread.
        unsafe { invoke_pattern.Invoke() }
            .map_err(|error| format!("invoke pattern action failed: {error}"))
    }

    pub(super) fn do_legacy_default_action(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationLegacyIAccessiblePattern, UIA_LegacyIAccessiblePatternId,
        };

        // SAFETY: The element comes from the current UI Automation traversal; pattern lookup and
        // action failures are converted into smoke-driver errors.
        let legacy_pattern: IUIAutomationLegacyIAccessiblePattern = unsafe {
            element
                .GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId)
                .map_err(|error| format!("legacy accessible pattern unavailable: {error}"))?
        };
        // SAFETY: The legacy pattern was obtained from the active UI Automation element and is
        // used immediately on the same thread.
        unsafe { legacy_pattern.DoDefaultAction() }
            .map_err(|error| format!("legacy default action failed: {error}"))
    }

    pub(super) fn select_automation_element(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationSelectionItemPattern, UIA_SelectionItemPatternId,
        };

        // SAFETY: The element comes from the current UI Automation traversal; pattern lookup and
        // selection failures are converted into smoke-driver errors.
        let selection_pattern: IUIAutomationSelectionItemPattern = unsafe {
            element
                .GetCurrentPatternAs(UIA_SelectionItemPatternId)
                .map_err(|error| format!("selection-item pattern unavailable: {error}"))?
        };
        // SAFETY: The selection-item pattern was obtained from the active UI Automation element
        // and is used immediately on the same thread.
        unsafe { selection_pattern.Select() }
            .map_err(|error| format!("selection-item action failed: {error}"))
    }

    pub(super) fn toggle_automation_element(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{IUIAutomationTogglePattern, UIA_TogglePatternId};

        // SAFETY: The element comes from the current UI Automation traversal; pattern lookup and
        // toggle failures are converted into smoke-driver errors.
        let toggle_pattern: IUIAutomationTogglePattern = unsafe {
            element
                .GetCurrentPatternAs(UIA_TogglePatternId)
                .map_err(|error| format!("toggle pattern unavailable: {error}"))?
        };
        // SAFETY: The toggle pattern was obtained from the active UI Automation element and is
        // used immediately on the same thread.
        unsafe { toggle_pattern.Toggle() }.map_err(|error| format!("toggle action failed: {error}"))
    }

    pub(super) fn expand_automation_element(
        element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<(), String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationExpandCollapsePattern, UIA_ExpandCollapsePatternId,
        };

        // SAFETY: The element comes from the current UI Automation traversal; pattern lookup and
        // expand failures are converted into smoke-driver errors.
        let expand_pattern: IUIAutomationExpandCollapsePattern = unsafe {
            element
                .GetCurrentPatternAs(UIA_ExpandCollapsePatternId)
                .map_err(|error| format!("expand-collapse pattern unavailable: {error}"))?
        };
        // SAFETY: The expand-collapse pattern was obtained from the active UI Automation element
        // and is used immediately on the same thread.
        unsafe { expand_pattern.Expand() }.map_err(|error| format!("expand action failed: {error}"))
    }

    pub(super) fn collect_subtree_elements(
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<windows::Win32::UI::Accessibility::IUIAutomationElementArray, String> {
        use windows::Win32::UI::Accessibility::TreeScope_Subtree;

        // SAFETY: `automation` is created inside `with_ui_automation` after COM initialization.
        // The condition object is used immediately for the subtree query.
        let all_condition = unsafe {
            automation.CreateTrueCondition().map_err(|error| {
                format!("failed to create UI Automation true condition: {error}")
            })?
        };
        // SAFETY: `root` and `all_condition` come from the active UI Automation COM scope. The
        // returned array is kept within the driver callback that owns that scope.
        unsafe {
            root.FindAll(TreeScope_Subtree, &all_condition)
                .map_err(|error| {
                    format!("failed to enumerate UI Automation element subtree: {error}")
                })
        }
    }

    pub(super) fn collect_accessible_names(
        window: PlatformWindowHandle,
    ) -> Result<Vec<String>, String> {
        Self::with_ui_automation(window, "accessibility discovery", |automation, root| {
            let elements = Self::collect_subtree_elements(automation, root)?;
            let length = Self::automation_element_count(&elements)?;
            let mut names = Vec::new();
            for index in 0..length {
                let element = Self::automation_element_at_required(&elements, index)?;
                let trimmed = Self::automation_element_name_required(&element)?;
                if !trimmed.is_empty() {
                    names.push(trimmed);
                }
            }
            names.sort();
            names.dedup();
            Ok(names)
        })
    }
}
