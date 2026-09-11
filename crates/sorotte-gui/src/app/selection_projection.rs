//! Selection and playback menu projection shared by feature consumers.
use super::shell_state::*;
pub(super) fn apply_selection_to_surfaces(
    selection: &GuiSelectionState,
    main_window: &mut MainWindowShellState,
    menus: &mut MenuDialogShellState,
    media_search: &mut MediaSearchWorkflowShellState,
) {
    for (index, user) in main_window.users.iter_mut().enumerate() {
        user.is_selected = selection.selected_main_window_user == Some(index);
    }
    for (index, item) in main_window.playlist.iter_mut().enumerate() {
        item.is_selected = selection.selected_main_window_playlist == Some(index);
    }
    for (section_index, section) in menus.sections.iter_mut().enumerate() {
        for (action_index, action) in section.actions.iter_mut().enumerate() {
            action.is_selected =
                selection.selected_menu_action == Some((section_index, action_index));
        }
    }
    for (index, directory) in media_search.directories.iter_mut().enumerate() {
        directory.is_selected = selection.selected_media_search_directory == Some(index);
    }
}

pub(super) fn normalize_selected_menu_action_after_runtime_update(
    menus: &MenuDialogShellState,
    selection: &mut GuiSelectionState,
) {
    let Some((selected_section_index, selected_action_index)) = selection.selected_menu_action
    else {
        return;
    };
    if menus
        .sections
        .get(selected_section_index)
        .and_then(|section| section.actions.get(selected_action_index))
        .is_some_and(|action| action.enabled)
    {
        return;
    }

    let replacement_in_section = menus
        .sections
        .get(selected_section_index)
        .and_then(|section| {
            section
                .actions
                .iter()
                .position(|action| action.enabled)
                .map(|action_index| (selected_section_index, action_index))
        });
    selection.selected_menu_action = replacement_in_section.or_else(|| {
        menus
            .sections
            .iter()
            .enumerate()
            .find_map(|(section_index, section)| {
                section
                    .actions
                    .iter()
                    .position(|action| action.enabled)
                    .map(|action_index| (section_index, action_index))
            })
    });
}

pub(super) fn sync_playback_menu_actions_from_runtime_state(
    main_window: &MainWindowShellState,
    menus: &mut MenuDialogShellState,
    selection: &mut GuiSelectionState,
    pending_operation: &Option<GuiPendingOperationState>,
    can_toggle_pause: bool,
) {
    let mut set_enabled = |id, enabled| {
        if let Some(action) = menus.action_mut(id) {
            action.enabled = enabled;
        }
    };
    let busy = pending_operation.is_some();
    let playback_controls_available = !busy && !main_window.playlist.is_empty();
    let can_open_media_file = !busy
        && (main_window.playback.can_toggle_pause
            || main_window.playback.can_seek
            || main_window.playback.can_manage_playlist);
    set_enabled(MenuActionId::OpenMedia, can_open_media_file);
    set_enabled(
        MenuActionId::Play,
        playback_controls_available && can_toggle_pause,
    );
    set_enabled(
        MenuActionId::Pause,
        playback_controls_available && can_toggle_pause,
    );
    set_enabled(
        MenuActionId::TogglePause,
        playback_controls_available && can_toggle_pause,
    );
    set_enabled(
        MenuActionId::Seek,
        playback_controls_available && main_window.playback.can_seek,
    );
    set_enabled(
        MenuActionId::UndoSeek,
        playback_controls_available && main_window.playback.can_undo_seek,
    );
    set_enabled(
        MenuActionId::SharedPlaylist,
        !busy && main_window.playback.can_manage_playlist,
    );
    set_enabled(
        MenuActionId::SetOffset,
        playback_controls_available && main_window.playback.can_set_offset,
    );
    normalize_selected_menu_action_after_runtime_update(menus, selection);
}
pub(super) fn default_selection_from_surfaces(
    main_window: &MainWindowShellState,
    media_search: &MediaSearchWorkflowShellState,
    menus: &MenuDialogShellState,
    selection: &mut GuiSelectionState,
    main_window_playlist_selection_is_local: &mut bool,
) {
    selection.selected_main_window_user = (!main_window.users.is_empty()).then_some(0);
    selection.selected_main_window_playlist = main_window
        .playlist
        .iter()
        .position(|row| row.is_selected)
        .or_else(|| (!main_window.playlist.is_empty()).then_some(0));
    *main_window_playlist_selection_is_local =
        false && selection.selected_main_window_playlist.is_some();
    selection.selected_menu_action =
        menus
            .sections
            .iter()
            .enumerate()
            .find_map(|(section_index, section)| {
                (!section.actions.is_empty()).then_some((section_index, 0))
            });
    selection.selected_media_search_directory = (!media_search.directories.is_empty()).then_some(0);
}

pub(super) fn normalize_selection(
    main_window: &MainWindowShellState,
    media_search: &MediaSearchWorkflowShellState,
    menus: &MenuDialogShellState,
    selection: &mut GuiSelectionState,
    main_window_playlist_selection_is_local: &mut bool,
) {
    if selection
        .selected_main_window_user
        .is_some_and(|index| index >= main_window.users.len())
    {
        selection.selected_main_window_user = (!main_window.users.is_empty()).then_some(0);
    }
    if selection
        .selected_main_window_playlist
        .is_some_and(|index| index >= main_window.playlist.len())
    {
        selection.selected_main_window_playlist = (!main_window.playlist.is_empty()).then_some(0);
        *main_window_playlist_selection_is_local =
            false && selection.selected_main_window_playlist.is_some();
    }
    if selection
        .selected_menu_action
        .is_some_and(|(section_index, action_index)| {
            menus
                .sections
                .get(section_index)
                .is_none_or(|section| action_index >= section.actions.len())
        })
    {
        selection.selected_menu_action =
            menus
                .sections
                .iter()
                .enumerate()
                .find_map(|(section_index, section)| {
                    (!section.actions.is_empty()).then_some((section_index, 0))
                });
    }
    if selection
        .selected_media_search_directory
        .is_some_and(|index| index >= media_search.directories.len())
    {
        selection.selected_media_search_directory =
            (!media_search.directories.is_empty()).then_some(0);
    }
}
