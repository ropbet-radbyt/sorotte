use super::*;

#[test]
fn parse_local_input_command_noarg_aliases_ignore_extra_parameters_legacy_style() {
    assert_eq!(
        parse_local_input_command("help now"),
        Some(LocalInputCommand::ShowHelp)
    );
    assert_eq!(
        parse_local_input_command("list now"),
        Some(LocalInputCommand::RequestUserList)
    );
    assert_eq!(
        parse_local_input_command("playlist now"),
        Some(LocalInputCommand::ShowPlaylist)
    );
    assert_eq!(
        parse_local_input_command("next now"),
        Some(LocalInputCommand::NextPlaylistItem)
    );
    assert_eq!(
        parse_local_input_command("toggle now"),
        Some(LocalInputCommand::ToggleReady)
    );
    assert_eq!(
        parse_local_input_command("p now"),
        Some(LocalInputCommand::TogglePause)
    );
    assert_eq!(
        parse_local_input_command("undo now"),
        Some(LocalInputCommand::UndoSeek)
    );
    assert_eq!(
        parse_local_input_command("undoplaylist now"),
        Some(LocalInputCommand::UndoPlaylistChange)
    );
    assert_eq!(
        parse_local_input_command("shuffleremainingplaylist now"),
        Some(LocalInputCommand::ShuffleRemainingPlaylist)
    );
    assert_eq!(
        parse_local_input_command("shuffleentireplaylist now"),
        Some(LocalInputCommand::ShuffleEntirePlaylist)
    );
}

#[test]
fn parse_local_input_command_noarg_aliases_require_literal_space_delimiter() {
    assert_eq!(
        parse_local_input_command("/help\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/list\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/playlist\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/next\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/pause\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/toggle\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/undo\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/undoplaylist\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_known_tokens_with_tab_delimiter_show_unknown_help() {
    assert_eq!(
        parse_local_input_command("help\tplease"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("chat\thello"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("queue\tmovie.mkv"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("room\troom2"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("setready\tbob"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek\t1:30"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_known_tokens_with_non_space_delimiters_show_unknown_help() {
    for delimiter in ["\t", "\u{000B}", "\u{000C}"] {
        for (token, payload) in [
            ("help", "please"),
            ("chat", "hello"),
            ("queue", "movie.mkv"),
            ("room", "room2"),
            ("setready", "bob"),
            ("seek", "1:30"),
            ("offset", "1:30"),
            ("auth", "AB-123-456"),
            ("create", "roomx"),
            ("toggle", "now"),
            ("list", "all"),
            ("/help", "please"),
            ("/chat", "hello"),
            ("/queue", "movie.mkv"),
            ("/room", "room2"),
            ("/setready", "bob"),
            ("/seek", "1:30"),
            ("/offset", "1:30"),
            ("/auth", "AB-123-456"),
            ("/create", "roomx"),
            ("/toggle", "now"),
            ("/list", "all"),
        ] {
            let input = format!("{token}{delimiter}{payload}");
            assert_eq!(
                parse_local_input_command(&input),
                Some(LocalInputCommand::ShowUnknownCommandHelp),
                "expected unknown-help for token {token:?} with delimiter U+{:04X}",
                delimiter.chars().next().unwrap_or_default() as u32
            );
        }
    }
}

#[test]
fn parse_local_input_command_known_aliases_with_leading_tab_show_unknown_help() {
    assert_eq!(
        parse_local_input_command("\thelp"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\tlist"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\ttoggle"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\tp"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\thello everyone"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\t/chat hello everyone"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\u{000B}help"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("\u{000B}hello everyone"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}
