use super::*;

#[test]
fn parse_local_input_command_parses_pause_aliases() {
    assert_eq!(
        parse_local_input_command("pause"),
        Some(LocalInputCommand::Pause)
    );
    assert_eq!(
        parse_local_input_command("play"),
        Some(LocalInputCommand::Play)
    );
    assert_eq!(
        parse_local_input_command("p"),
        Some(LocalInputCommand::TogglePause)
    );
    assert_eq!(
        parse_local_input_command("/pause"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/play"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/p"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_seek_aliases() {
    assert_eq!(
        parse_local_input_command("seek 90"),
        Some(LocalInputCommand::SeekAbsolute(90.0))
    );
    assert_eq!(
        parse_local_input_command("s 1:30"),
        Some(LocalInputCommand::SeekAbsolute(90.0))
    );
    assert_eq!(
        parse_local_input_command("/seek +0:10"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/s -2:00"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("s+0:10"),
        Some(LocalInputCommand::SeekRelative(10.0))
    );
    assert_eq!(
        parse_local_input_command("seek-2:00"),
        Some(LocalInputCommand::SeekRelative(-120.0))
    );
    assert_eq!(
        parse_local_input_command("+0:05"),
        Some(LocalInputCommand::SeekRelative(5.0))
    );
    assert_eq!(
        parse_local_input_command("1:30"),
        Some(LocalInputCommand::SeekAbsolute(90.0))
    );
    assert_eq!(
        parse_local_input_command("s 1 30"),
        Some(LocalInputCommand::SeekAbsolute(90.0))
    );
    assert_eq!(
        parse_local_input_command("seek 1h02m03"),
        Some(LocalInputCommand::SeekAbsolute(3723.0))
    );
    assert_eq!(
        parse_local_input_command("seek 1234"),
        Some(LocalInputCommand::SeekAbsolute(1234.0))
    );
    assert_eq!(
        parse_local_input_command("seek 1.123"),
        Some(LocalInputCommand::SeekAbsolute(1.123))
    );
    assert_eq!(
        parse_local_input_command("seek 12:123456"),
        Some(LocalInputCommand::SeekAbsolute(124176.0))
    );
    assert_eq!(
        parse_local_input_command("+1-30"),
        Some(LocalInputCommand::SeekRelative(90.0))
    );
    assert_eq!(
        parse_local_input_command("seek"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("s"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek nope"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek  +0:10"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek 90 "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek 1::30"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek 12345"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek 1.1234"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("seek 12:1234567"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("s+oops"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_offset_aliases() {
    assert_eq!(
        parse_local_input_command("offset 1:30"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Absolute(90.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("o +0:10"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Relative(10.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("o-2:00"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Relative(-120.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset /0:30"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::RelativeFromCurrentPositionMinus(30.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("o 1 30"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Absolute(90.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset +1-30"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Relative(90.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset /1h2m3"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::RelativeFromCurrentPositionMinus(3723.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset 123456789"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Absolute(123456789.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset 1.123"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Absolute(1.123)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset 12:123456789"),
        Some(LocalInputCommand::SetUserOffset(
            LocalOffsetCommand::Absolute(123457509.0)
        ))
    );
    assert_eq!(
        parse_local_input_command("offset"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("o"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("offset nope"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("offset  +0:10"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("offset 1:30 "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("offset 1234567890"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("offset 1.1234"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("offset 12:1234567890"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("o+oops"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_room_aliases() {
    assert_eq!(
        parse_local_input_command("room room2"),
        Some(LocalInputCommand::SetRoom("room2".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("room  room2  "),
        Some(LocalInputCommand::SetRoom(" room2  ".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("room   "),
        Some(LocalInputCommand::SetRoom("  ".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("room "),
        Some(LocalInputCommand::SetRoomWithLegacyFallback)
    );
    assert_eq!(
        parse_local_input_command("r room2"),
        Some(LocalInputCommand::SetRoom("room2".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("r "),
        Some(LocalInputCommand::SetRoomWithLegacyFallback)
    );
    assert_eq!(
        parse_local_input_command("/room room2"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/room "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/r room2"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/r "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("room"),
        Some(LocalInputCommand::SetRoomWithLegacyFallback)
    );
    assert_eq!(
        parse_local_input_command("r"),
        Some(LocalInputCommand::SetRoomWithLegacyFallback)
    );
    assert_eq!(
        parse_local_input_command("/room"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/r"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
}

#[test]
fn parse_local_input_command_parses_chat_and_unknown_slash_command_help() {
    assert_eq!(
        parse_local_input_command("hello everyone"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("chat hello everyone"),
        Some(LocalInputCommand::Chat("hello everyone".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("/ch hello"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("chat"),
        Some(LocalInputCommand::Chat("".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("ch"),
        Some(LocalInputCommand::Chat("".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("/chat"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/msg"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("chat  "),
        Some(LocalInputCommand::Chat(" ".to_owned()))
    );
    assert_eq!(
        parse_local_input_command("/msg   hello  "),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/unknown hello"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("/unknown\thello"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(
        parse_local_input_command("hello\teveryone"),
        Some(LocalInputCommand::ShowUnknownCommandHelp)
    );
    assert_eq!(parse_local_input_command(" hello everyone"), None);
    assert_eq!(parse_local_input_command(" /chat hello"), None);
    assert_eq!(parse_local_input_command(" /unknown hello"), None);
}
