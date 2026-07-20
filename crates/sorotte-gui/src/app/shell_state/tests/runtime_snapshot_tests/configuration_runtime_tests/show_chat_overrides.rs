use super::*;

#[test]
fn gui_configuration_does_not_expose_unimplemented_view_commands() {
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettingsMvp {
        chat_input_enabled: Some(true),
        chat_output_enabled: Some(true),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettingsMvp::default()
    });

    let visible_labels = state
        .menus
        .sections
        .iter()
        .flat_map(|section| section.actions.iter())
        .map(|action| action.label)
        .collect::<Vec<_>>();
    assert!(!visible_labels.contains(&"Show Chat"));
    assert!(!visible_labels.contains(&"Show Playlist"));
    assert!(!visible_labels.contains(&"Show Users"));
}
