use super::super::shell_state::{GuiShellAction, GuiShellModal, SorotteGuiShellAppState};
use super::super::widget_tree::{GuiWidgetKind, GuiWidgetNode};
use super::GuiWidgetEguiRenderer;

impl GuiWidgetEguiRenderer {
    pub(in crate::app) fn modal_window_title(modal: GuiShellModal) -> &'static str {
        match modal {
            GuiShellModal::TlsCertificatePrompt => "TLS Certificate Prompt",
            GuiShellModal::UpdateNotice => "Update Notice",
            GuiShellModal::About => "About Sorotte",
            GuiShellModal::PlayerSetup => "mpv Setup Required",
            GuiShellModal::StreamSupport => "Stream Support",
        }
    }

    pub(super) fn modal_body_lines(
        modal: GuiShellModal,
        state: &SorotteGuiShellAppState,
    ) -> Vec<String> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                "A TLS certificate prompt is active for the current connection.".to_owned(),
                "Trust the certificate for this session or reject it to keep the warning visible."
                    .to_owned(),
            ],
            GuiShellModal::UpdateNotice => state
                .update_check
                .body_lines(Some(state.runtime_language_tag_legacy_compatible())),
            GuiShellModal::About => vec![
                "The reducer reports that the About dialog is open.".to_owned(),
                "This modal now routes into the existing help and update actions.".to_owned(),
            ],
            GuiShellModal::PlayerSetup => {
                let mut lines = vec![
                    state
                        .player_setup_issue_title()
                        .unwrap_or("mpv setup issue")
                        .to_owned(),
                    state
                        .player_setup_issue_summary()
                        .unwrap_or("Sorotte needs mpv before playback can start.")
                        .to_owned(),
                ];
                if let Some(issue) = state.player_setup_issue.as_ref() {
                    lines.push(issue.message.clone());
                }
                if state.connect_blocked_by_player_setup_issue()
                    && let Some(message) = state.player_setup_connect_block_message()
                {
                    lines.push(message);
                }
                lines
            }
            GuiShellModal::StreamSupport => {
                let mut lines = vec![
                    state.stream_helper_status_title().to_owned(),
                    state.stream_helper_status_summary(),
                ];
                if let Some(install_location) = state.stream_helper.install_location.as_ref() {
                    lines.push(format!("Install location: {install_location}"));
                }
                if let Some(downloader_status) = state.stream_helper.downloader_status.as_ref() {
                    lines.push(format!("yt-dlp: {downloader_status}"));
                }
                if let Some(js_runtime_status) = state.stream_helper.js_runtime_status.as_ref() {
                    lines.push(format!("Deno: {js_runtime_status}"));
                }
                if let Some(target) = state.stream_helper.target.as_ref() {
                    lines.push(format!("Target: {target}"));
                }
                if let Some(message) = state.stream_helper.message.as_ref() {
                    lines.push(message.clone());
                }
                if state.stream_helper_remediation.active {
                    if let Some(label) = state.stream_helper_remediation.label.as_ref() {
                        lines.push(format!("Progress: {label}"));
                    }
                    if let Some(detail) = state.stream_helper_remediation.detail.as_ref() {
                        lines.push(detail.clone());
                    }
                }
                if state.stream_helper.integration_supported {
                    lines.push(
                        "Import yt-dlp or Deno to copy existing helper binaries into Sorotte's managed stream-helper directory."
                            .to_owned(),
                    );
                }
                lines
            }
        }
    }

    pub(in crate::app) fn modal_actions(modal: GuiShellModal) -> Vec<(&'static str, &'static str)> {
        match modal {
            GuiShellModal::TlsCertificatePrompt => vec![
                ("shell:modal:tls:trust", "Trust Certificate"),
                ("shell:modal:tls:reject", "Reject Certificate"),
                ("shell:modal:tls:help", "Open Help"),
            ],
            GuiShellModal::UpdateNotice => vec![
                ("shell:modal:update:check-again", "Check Again"),
                ("shell:modal:update:download", "Download Update"),
                ("shell:modal:update:restart", "Restart to Update"),
            ],
            GuiShellModal::About => vec![
                ("shell:modal:about:help", "Open Help"),
                ("shell:modal:about:update", "Check for Updates"),
            ],
            GuiShellModal::PlayerSetup => vec![
                ("shell:modal:player-setup:autodetect", "Auto-detect mpv"),
                ("shell:modal:player-setup:choose-path", "Choose mpv.exe"),
                ("shell:modal:player-setup:retry", "Retry mpv"),
                ("shell:modal:player-setup:open-settings", "Open Settings"),
            ],
            GuiShellModal::StreamSupport => vec![
                ("shell:modal:stream-support:install", "Install Helper"),
                (
                    "shell:modal:stream-support:import-downloader",
                    "Import yt-dlp",
                ),
                (
                    "shell:modal:stream-support:import-js-runtime",
                    "Import Deno",
                ),
                (
                    "shell:modal:stream-support:open-location",
                    "Open Install Location",
                ),
                ("shell:modal:stream-support:recheck", "Recheck Support"),
                ("shell:modal:stream-support:retry", "Retry URL"),
                ("shell:modal:stream-support:open-settings", "Open Plugins"),
            ],
        }
    }

    pub(in crate::app) fn modal_action_enabled(state: &SorotteGuiShellAppState, id: &str) -> bool {
        match id {
            "shell:modal:update:check-again" => !matches!(
                state.update_check.status,
                Some(super::super::remote_services::LegacyUpdateCheckStatus::Checking)
            ),
            "shell:modal:update:download" => state.update_check.can_download_update(),
            "shell:modal:update:restart" => state.update_check.can_restart_to_update(),
            "shell:modal:player-setup:autodetect"
            | "shell:modal:player-setup:choose-path"
            | "shell:modal:player-setup:open-settings" => state.pending_operation.is_none(),
            "shell:modal:player-setup:retry" => {
                state.pending_operation.is_none() && state.player_setup_retry_available()
            }
            "shell:modal:stream-support:install" => {
                state.pending_operation.is_none()
                    && !state.stream_helper_remediation.active
                    && state.stream_helper.install_supported
            }
            "shell:modal:stream-support:import-downloader"
            | "shell:modal:stream-support:import-js-runtime" => {
                state.pending_operation.is_none()
                    && !state.stream_helper_remediation.active
                    && state.stream_helper.integration_supported
            }
            "shell:modal:stream-support:open-location" => {
                state.stream_helper.open_install_location_available
            }
            "shell:modal:stream-support:recheck" => {
                state.pending_operation.is_none() && !state.stream_helper_remediation.active
            }
            "shell:modal:stream-support:retry" => {
                state.pending_operation.is_none()
                    && !state.stream_helper_remediation.active
                    && state.stream_helper.retry_available
            }
            "shell:modal:stream-support:open-settings" => {
                state.pending_operation.is_none() && !state.stream_helper_remediation.active
            }
            _ => true,
        }
    }

    pub(in crate::app) fn modal_close_enabled(
        state: &SorotteGuiShellAppState,
        modal: GuiShellModal,
    ) -> bool {
        match modal {
            GuiShellModal::PlayerSetup => !state.connect_blocked_by_player_setup_issue(),
            GuiShellModal::TlsCertificatePrompt
            | GuiShellModal::UpdateNotice
            | GuiShellModal::About
            | GuiShellModal::StreamSupport => true,
        }
    }

    pub(super) fn modal_button_actions(
        state: &SorotteGuiShellAppState,
        id: &str,
        label: &str,
    ) -> Vec<GuiShellAction> {
        let node = GuiWidgetNode::leaf(id, label, GuiWidgetKind::Button, None, true, false);
        Self::actions_for_clicked_button(state, &node)
    }
}
