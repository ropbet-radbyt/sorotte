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
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
        }

        let _com = ComScope::initialize(operation_label)?;
        let automation: IUIAutomation = unsafe {
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map_err(|error| format!("failed to create IUIAutomation instance: {error}"))?
        };
        let root = unsafe {
            automation
                .ElementFromHandle(HWND(window))
                .map_err(|error| {
                    format!("failed to map main window handle into UI Automation element: {error}")
                })?
        };
        run(&automation, &root)
    }

    pub(super) fn collect_subtree_elements(
        automation: &windows::Win32::UI::Accessibility::IUIAutomation,
        root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) -> Result<windows::Win32::UI::Accessibility::IUIAutomationElementArray, String> {
        use windows::Win32::UI::Accessibility::TreeScope_Subtree;

        let all_condition = unsafe {
            automation.CreateTrueCondition().map_err(|error| {
                format!("failed to create UI Automation true condition: {error}")
            })?
        };
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
            let length = unsafe {
                elements.Length().map_err(|error| {
                    format!("failed to read UI Automation element count: {error}")
                })?
            };
            let mut names = Vec::new();
            for index in 0..length {
                let element = unsafe {
                    elements.GetElement(index).map_err(|error| {
                        format!("failed to fetch UI Automation element at index {index}: {error}")
                    })?
                };
                let name = unsafe {
                    element.CurrentName().map_err(|error| {
                        format!("failed to read UI Automation element name: {error}")
                    })?
                };
                let trimmed = name.to_string().trim().to_owned();
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
