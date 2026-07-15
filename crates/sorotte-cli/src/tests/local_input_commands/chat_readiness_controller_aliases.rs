use super::*;

#[test]
fn parse_local_input_chat_message_handles_plain_and_prefixed_inputs() {
    assert_eq!(
        parse_local_input_chat_message("chat hello everyone"),
        Some("hello everyone".to_owned())
    );
    assert_eq!(
        parse_local_input_chat_message("ch hello everyone"),
        Some("hello everyone".to_owned())
    );
    assert_eq!(
        parse_local_input_chat_message("chat   hello everyone  "),
        Some("  hello everyone  ".to_owned())
    );
    assert_eq!(
        parse_local_input_chat_message("chat hello\teveryone"),
        Some("hello\teveryone".to_owned())
    );
}

#[test]
fn parse_local_input_chat_message_handles_empty_chat_aliases_and_ignores_unknown_commands() {
    assert_eq!(parse_local_input_chat_message(""), None);
    assert_eq!(parse_local_input_chat_message("   "), None);
    assert_eq!(parse_local_input_chat_message("hello everyone"), None);
    assert_eq!(parse_local_input_chat_message(" hello everyone"), None);
    assert_eq!(parse_local_input_chat_message("\thello everyone"), None);
    assert_eq!(
        parse_local_input_chat_message("\u{000B}hello everyone"),
        None
    );
    assert_eq!(
        parse_local_input_chat_message(" /chat hello everyone"),
        None
    );
    assert_eq!(parse_local_input_chat_message("chat"), Some("".to_owned()));
    assert_eq!(parse_local_input_chat_message("ch"), Some("".to_owned()));
    assert_eq!(parse_local_input_chat_message("/chat"), None);
    assert_eq!(parse_local_input_chat_message("/ch"), None);
    assert_eq!(parse_local_input_chat_message("/msg"), None);
    assert_eq!(
        parse_local_input_chat_message("chat  "),
        Some(" ".to_owned())
    );
    assert_eq!(parse_local_input_chat_message("/msg "), None);
    assert_eq!(parse_local_input_chat_message("/msg   "), None);
    assert_eq!(parse_local_input_chat_message("chat\thello"), None);
    assert_eq!(parse_local_input_chat_message("hello\teveryone"), None);
    assert_eq!(parse_local_input_chat_message("help\tplease"), None);
    assert_eq!(parse_local_input_chat_message("/unknown hello"), None);
}

#[test]
fn parse_local_input_command_parses_toggle_aliases() {
    assert_eq!(
        parse_local_input_command("toggle"),
        Some(LocalInputCommand::ToggleReady)
    );
    assert_eq!(
        parse_local_input_command("t"),
        Some(LocalInputCommand::ToggleReady)
    );
    assert_eq!(
        parse_local_input_command("/toggle"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/t"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_setready_aliases() {
    assert_eq!(
        parse_local_input_command("ready"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("ready bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("setready bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("sr bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("/setready bob"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/sr bob"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("setready"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("setready "),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("sr"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("/setready"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/sr"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("setready  "),
        Some(LocalInputCommand::SetUserReady {
            username: " ".to_owned(),
            ready: true
        })
    );
    assert_eq!(
        parse_local_input_command("setready   bob  "),
        Some(LocalInputCommand::SetUserReady {
            username: "  bob  ".to_owned(),
            ready: true
        })
    );
}

#[test]
fn parse_local_input_command_parses_setnotready_aliases() {
    assert_eq!(
        parse_local_input_command("not-ready"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("not-ready bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("setnotready bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("sn bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("snr bob"),
        Some(LocalInputCommand::SetUserReady {
            username: "bob".to_owned(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("/setnotready bob"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/sn bob"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/snr bob"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("setnotready"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("setnotready "),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("sn"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("snr"),
        Some(LocalInputCommand::SetUserReady {
            username: String::new(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("/setnotready"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/sn"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/snr"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("setnotready  "),
        Some(LocalInputCommand::SetUserReady {
            username: " ".to_owned(),
            ready: false
        })
    );
    assert_eq!(
        parse_local_input_command("setnotready   bob  "),
        Some(LocalInputCommand::SetUserReady {
            username: "  bob  ".to_owned(),
            ready: false
        })
    );
}

#[test]
fn parse_local_input_command_parses_create_aliases() {
    assert_eq!(
        parse_local_input_command("create"),
        Some(LocalInputCommand::CreateControlledRoom(None))
    );
    assert_eq!(
        parse_local_input_command("create "),
        Some(LocalInputCommand::CreateControlledRoom(None))
    );
    assert_eq!(
        parse_local_input_command("c"),
        Some(LocalInputCommand::CreateControlledRoom(None))
    );
    assert_eq!(
        parse_local_input_command("c "),
        Some(LocalInputCommand::CreateControlledRoom(None))
    );
    assert_eq!(
        parse_local_input_command("/create"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/create "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/c"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/c "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("create  "),
        Some(LocalInputCommand::CreateControlledRoom(Some(
            " ".to_owned()
        )))
    );
    assert_eq!(
        parse_local_input_command("create   base-room"),
        Some(LocalInputCommand::CreateControlledRoom(Some(
            "  base-room".to_owned()
        )))
    );
    assert_eq!(
        parse_local_input_command("create base-room"),
        Some(LocalInputCommand::CreateControlledRoom(Some(
            "base-room".to_owned()
        )))
    );
    assert_eq!(
        parse_local_input_command("c base-room"),
        Some(LocalInputCommand::CreateControlledRoom(Some(
            "base-room".to_owned()
        )))
    );
    assert_eq!(
        parse_local_input_command("/create base-room"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/c base-room"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_auth_aliases() {
    assert_eq!(
        parse_local_input_command("auth ab-123-456"),
        Some(LocalInputCommand::AuthController("ab-123-456".into()))
    );
    assert_eq!(
        parse_local_input_command("a ab-123-456"),
        Some(LocalInputCommand::AuthController("ab-123-456".into()))
    );
    assert_eq!(
        parse_local_input_command("/auth ab-123-456"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/a ab-123-456"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("auth"),
        Some(LocalInputCommand::AuthController(String::new().into()))
    );
    assert_eq!(
        parse_local_input_command("a"),
        Some(LocalInputCommand::AuthController(String::new().into()))
    );
    assert_eq!(
        parse_local_input_command("/auth"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/a"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("auth   "),
        Some(LocalInputCommand::AuthController(String::new().into()))
    );
}

#[test]
fn parse_local_input_command_parses_list_aliases() {
    assert_eq!(
        parse_local_input_command("list"),
        Some(LocalInputCommand::RequestUserList)
    );
    assert_eq!(
        parse_local_input_command("l"),
        Some(LocalInputCommand::RequestUserList)
    );
    assert_eq!(
        parse_local_input_command("users"),
        Some(LocalInputCommand::RequestUserList)
    );
    assert_eq!(
        parse_local_input_command("/list"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/l"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/users"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_help_aliases() {
    assert_eq!(
        parse_local_input_command("help"),
        Some(LocalInputCommand::ShowHelp)
    );
    assert_eq!(
        parse_local_input_command("h"),
        Some(LocalInputCommand::ShowHelp)
    );
    assert_eq!(
        parse_local_input_command("?"),
        Some(LocalInputCommand::ShowHelp)
    );
    assert_eq!(
        parse_local_input_command("/help"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/h"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/?"),
        Some(LocalInputCommand::ShowHelp)
    );
    assert_eq!(
        parse_local_input_command("\\?"),
        Some(LocalInputCommand::ShowHelp)
    );
}
