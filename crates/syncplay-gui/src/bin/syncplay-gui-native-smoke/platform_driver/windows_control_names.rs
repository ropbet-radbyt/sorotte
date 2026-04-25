use super::super::MAIN_WINDOW_LOCAL_READY_BUTTON_NAME;
use super::NativeControlKind;

impl NativeControlKind {
    pub(super) fn matches_control_type(
        self,
        control_type: windows::Win32::UI::Accessibility::UIA_CONTROLTYPE_ID,
    ) -> bool {
        use windows::Win32::UI::Accessibility::{
            UIA_ButtonControlTypeId, UIA_MenuItemControlTypeId,
        };

        match self {
            Self::Any => true,
            Self::Button => control_type == UIA_ButtonControlTypeId,
            Self::MenuItem => control_type == UIA_MenuItemControlTypeId,
        }
    }
}

pub(super) fn is_local_ready_button_request(name: &str) -> bool {
    name == MAIN_WINDOW_LOCAL_READY_BUTTON_NAME
}

fn is_local_ready_button_name(name: &str) -> bool {
    matches!(
        name,
        MAIN_WINDOW_LOCAL_READY_BUTTON_NAME | "Ready" | "Not Ready"
    )
}

pub(super) fn matches_control_name(requested_name: &str, current_name: &str) -> bool {
    if is_local_ready_button_request(requested_name) {
        is_local_ready_button_name(current_name)
    } else {
        current_name == requested_name
    }
}
