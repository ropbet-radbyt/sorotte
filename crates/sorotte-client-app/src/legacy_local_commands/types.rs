use sorotte_secret::SecretValue;

#[derive(Debug, Clone, PartialEq)]
pub enum LocalOffsetCommand {
    Absolute(f64),
    Relative(f64),
    RelativeFromCurrentPositionMinus(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocalInputCommand {
    Chat(String),
    RequestUserList,
    ShowUnknownCommandHelp,
    ShowHelp,
    ShowPlaylistInvalidIndexError,
    ShowQueueMissingFileError,
    ShowPlaylist,
    SelectPlaylistIndex(i64),
    NextPlaylistItem,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    KeepWaitingForSeekPreparation,
    JoinNearestBufferedSeekPreparation,
    CancelSeekPreparation,
    SetUserOffset(LocalOffsetCommand),
    SeekAbsolute(f64),
    SeekRelative(f64),
    TogglePause,
    ToggleReady,
    SetUserReady {
        username: String,
        ready: bool,
    },
    CreateControlledRoom(Option<String>),
    AuthController(SecretValue),
    SetRoomWithLegacyFallback,
    SetRoom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalInputCommandErrorKind {
    PlaylistInvalidIndex,
    QueueMissingFile,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedLocalRuntimeAction {
    SendChat(String),
    RequestUserList,
    SetPlaylistIndex(i64),
    AdvancePlaylistIndex,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    KeepWaitingForSeekPreparation,
    JoinNearestBufferedSeekPreparation,
    CancelSeekPreparation,
    SetUserOffset(LocalOffsetCommand),
    SeekToPosition(f64),
    SeekByOffset(f64),
    TogglePause,
    ToggleReady,
    SetUserReady {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: SecretValue,
    },
    SetRoomWithLegacyFallback(String),
    SetRoom(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedLocalRuntimeDispatch {
    pub line_to_emit: Option<String>,
    pub action: Option<PlannedLocalRuntimeAction>,
    pub updated_user_offset_seconds: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedLocalInputDispatch {
    Suppressed,
    EmitUnknownCommandHelp,
    EmitHelp,
    EmitError(LocalInputCommandErrorKind),
    EmitPlaylist,
    Run(PlannedLocalRuntimeAction),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlannedLocalInputCommand {
    SendChat(String),
    RequestUserList,
    ShowUnknownCommandHelp,
    ShowHelp,
    ShowError(LocalInputCommandErrorKind),
    ShowPlaylist,
    SelectPlaylistIndex(i64),
    NextPlaylistItem,
    QueuePlaylistItem {
        file_name: String,
        select_after_queue: bool,
    },
    DeletePlaylistIndex(i64),
    UndoPlaylistChange,
    ShuffleRemainingPlaylist,
    ShuffleEntirePlaylist,
    UndoSeek,
    KeepWaitingForSeekPreparation,
    JoinNearestBufferedSeekPreparation,
    CancelSeekPreparation,
    SetUserOffset(LocalOffsetCommand),
    SeekAbsolute(f64),
    SeekRelative(f64),
    TogglePause,
    ToggleReady,
    SetUserReady {
        username: String,
        ready: bool,
    },
    RequestControllerAuth {
        room: String,
        password: SecretValue,
    },
    SetRoomWithLegacyFallback(String),
    SetRoom(String),
}

pub struct LocalInputCommandPlanningContext<'a> {
    pub current_room: Option<&'a str>,
    pub configured_room: &'a str,
}
