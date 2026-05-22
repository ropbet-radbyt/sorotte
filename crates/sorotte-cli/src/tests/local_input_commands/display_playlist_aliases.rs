use super::*;

#[test]
fn localized_legacy_compatibility_headings_legacy_compatible_use_selected_language() {
    assert_eq!(
        crate::localized_legacy_startup_compatibility_heading_legacy_compatible(Some("fr")),
        "Compatibilite de demarrage de Legacy Python ConfigurationGetter :"
    );
    assert_eq!(
        crate::localized_legacy_ini_compatibility_heading_legacy_compatible(Some("de")),
        "Legacy-Python-ConfigurationGetter sorotte.ini-Kompatibilitaet:"
    );
    assert_eq!(
        crate::localized_compatibility_input_label_legacy_compatible(Some("es")),
        "Entrada"
    );
    assert_eq!(
        crate::localized_compatibility_note_label_legacy_compatible(Some("ko")),
        "Bigo"
    );
}

#[test]
fn parse_local_input_command_parses_playlist_aliases() {
    assert_eq!(
        parse_local_input_command("playlist"),
        Some(LocalInputCommand::ShowPlaylist)
    );
    assert_eq!(
        parse_local_input_command("ql"),
        Some(LocalInputCommand::ShowPlaylist)
    );
    assert_eq!(
        parse_local_input_command("pl"),
        Some(LocalInputCommand::ShowPlaylist)
    );
    assert_eq!(
        parse_local_input_command("/playlist"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/ql"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/pl"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_select_aliases() {
    assert_eq!(
        parse_local_input_command("select 1"),
        Some(LocalInputCommand::SelectPlaylistIndex(0))
    );
    assert_eq!(
        parse_local_input_command("qs 2"),
        Some(LocalInputCommand::SelectPlaylistIndex(1))
    );
    assert_eq!(
        parse_local_input_command("/select 3"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/qs 4"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("select"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
    assert_eq!(
        parse_local_input_command("qs"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
    assert_eq!(
        parse_local_input_command("/select"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/qs"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("select 0"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
}

#[test]
fn parse_local_input_command_parses_next_aliases() {
    assert_eq!(
        parse_local_input_command("next"),
        Some(LocalInputCommand::NextPlaylistItem)
    );
    assert_eq!(
        parse_local_input_command("qn"),
        Some(LocalInputCommand::NextPlaylistItem)
    );
    assert_eq!(
        parse_local_input_command("/next"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/qn"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_queue_aliases() {
    assert_eq!(
        parse_local_input_command("queue episode1.mkv"),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "episode1.mkv".to_owned(),
            select_after_queue: false
        })
    );
    assert_eq!(
        parse_local_input_command("queue  "),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: " ".to_owned(),
            select_after_queue: false
        })
    );
    assert_eq!(
        parse_local_input_command("queue   episode1.mkv  "),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "  episode1.mkv  ".to_owned(),
            select_after_queue: false
        })
    );
    assert_eq!(
        parse_local_input_command("qa episode2.mkv"),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "episode2.mkv".to_owned(),
            select_after_queue: false
        })
    );
    assert_eq!(
        parse_local_input_command("add episode3.mkv"),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "episode3.mkv".to_owned(),
            select_after_queue: false
        })
    );
    assert_eq!(
        parse_local_input_command("/queue episode4.mkv"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("queue"),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
    assert_eq!(
        parse_local_input_command("queue "),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
    assert_eq!(
        parse_local_input_command("qa"),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
    assert_eq!(
        parse_local_input_command("add"),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
}

#[test]
fn parse_local_input_command_parses_queueandselect_aliases() {
    assert_eq!(
        parse_local_input_command("queueandselect episode1.mkv"),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "episode1.mkv".to_owned(),
            select_after_queue: true
        })
    );
    assert_eq!(
        parse_local_input_command("queueandselect  "),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: " ".to_owned(),
            select_after_queue: true
        })
    );
    assert_eq!(
        parse_local_input_command("queueandselect   episode1.mkv  "),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "  episode1.mkv  ".to_owned(),
            select_after_queue: true
        })
    );
    assert_eq!(
        parse_local_input_command("qas episode2.mkv"),
        Some(LocalInputCommand::QueuePlaylistItem {
            file_name: "episode2.mkv".to_owned(),
            select_after_queue: true
        })
    );
    assert_eq!(
        parse_local_input_command("/queueandselect episode3.mkv"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/qas episode4.mkv"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("queueandselect"),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
    assert_eq!(
        parse_local_input_command("queueandselect "),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
    assert_eq!(
        parse_local_input_command("qas"),
        Some(LocalInputCommand::ShowQueueMissingFileError)
    );
}

#[test]
fn parse_local_input_command_parses_delete_aliases() {
    assert_eq!(
        parse_local_input_command("delete 1"),
        Some(LocalInputCommand::DeletePlaylistIndex(0))
    );
    assert_eq!(
        parse_local_input_command("d 2"),
        Some(LocalInputCommand::DeletePlaylistIndex(1))
    );
    assert_eq!(
        parse_local_input_command("qd 3"),
        Some(LocalInputCommand::DeletePlaylistIndex(2))
    );
    assert_eq!(
        parse_local_input_command("/delete 4"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/d 5"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/qd 6"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("delete"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
    assert_eq!(
        parse_local_input_command("d"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
    assert_eq!(
        parse_local_input_command("qd"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
    assert_eq!(
        parse_local_input_command("delete 0"),
        Some(LocalInputCommand::ShowPlaylistInvalidIndexError)
    );
}

#[test]
fn parse_local_input_command_parses_playlist_undo_aliases() {
    assert_eq!(
        parse_local_input_command("undoplaylist"),
        Some(LocalInputCommand::UndoPlaylistChange)
    );
    assert_eq!(
        parse_local_input_command("/undoplaylist"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_shuffle_playlist_aliases() {
    assert_eq!(
        parse_local_input_command("shuffleremainingplaylist"),
        Some(LocalInputCommand::ShuffleRemainingPlaylist)
    );
    assert_eq!(
        parse_local_input_command("/shuffleremainingplaylist"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("shuffleentireplaylist"),
        Some(LocalInputCommand::ShuffleEntirePlaylist)
    );
    assert_eq!(
        parse_local_input_command("/shuffleentireplaylist"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_undo_aliases() {
    assert_eq!(
        parse_local_input_command("undo"),
        Some(LocalInputCommand::UndoSeek)
    );
    assert_eq!(
        parse_local_input_command("u"),
        Some(LocalInputCommand::UndoSeek)
    );
    assert_eq!(
        parse_local_input_command("revert"),
        Some(LocalInputCommand::UndoSeek)
    );
    assert_eq!(
        parse_local_input_command("/undo"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/u"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/revert"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}
