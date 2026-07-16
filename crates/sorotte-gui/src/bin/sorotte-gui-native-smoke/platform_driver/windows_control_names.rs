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

pub(super) fn matches_control_identity(
    requested_identity: &str,
    current_name: &str,
    automation_id: &str,
) -> bool {
    if is_local_ready_button_request(requested_identity) {
        is_local_ready_button_name(current_name)
    } else {
        current_name == requested_identity || automation_id == requested_identity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_identity_accepts_stable_automation_id_without_changing_visible_name() {
        assert!(matches_control_identity(
            "settings.connection.host",
            "Host",
            "settings.connection.host"
        ));
        assert!(!matches_control_identity(
            "settings.connection.host",
            "Host",
            "settings.connection.port"
        ));
    }

    #[test]
    fn control_identity_preserves_visible_name_and_dynamic_ready_matching() {
        assert!(matches_control_identity("Host", "Host", ""));
        assert!(matches_control_identity(
            MAIN_WINDOW_LOCAL_READY_BUTTON_NAME,
            "Not Ready",
            ""
        ));
    }
}
