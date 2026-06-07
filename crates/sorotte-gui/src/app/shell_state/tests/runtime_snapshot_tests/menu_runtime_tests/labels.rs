use super::*;

#[test]
fn gui_pending_operation_kind_labels_are_stable() {
    let labels = [
        GuiPendingOperationKind::SaveConfiguration.label(),
        GuiPendingOperationKind::ResetConfiguration.label(),
        GuiPendingOperationKind::ReloadConfiguration.label(),
        GuiPendingOperationKind::ConnectPublicServer.label(),
        GuiPendingOperationKind::RefreshPublicServers.label(),
        GuiPendingOperationKind::SearchMissingMedia.label(),
        GuiPendingOperationKind::SetPlaybackPause(true).label(),
        GuiPendingOperationKind::SetPlaybackPause(false).label(),
        GuiPendingOperationKind::TogglePlaybackPause.label(),
        GuiPendingOperationKind::SendChatMessage.label(),
    ];

    assert_eq!(
        labels,
        [
            "save-configuration",
            "reset-configuration",
            "reload-configuration",
            "connect-public-server",
            "refresh-public-servers",
            "search-missing-media",
            "pause-playback",
            "resume-playback",
            "toggle-playback-pause",
            "send-chat-message",
        ]
    );
}
