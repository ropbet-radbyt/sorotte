use sorotte_secret::SecretValue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoplayCountdownNotification {
    pub ready_user_count: usize,
    pub seconds_left: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReconnectTransitionNotification {
    Attempting {
        retries: u32,
        delay_seconds: f64,
    },
    Connected,
    Disconnected,
    RestoringState,
    StateRestoreValidationMismatch {
        local_paused: bool,
        room_paused: bool,
        local_position: f64,
        room_position: f64,
        position_diff_seconds: f64,
    },
    StateRestoreValidationCorrectionRetryScheduled {
        attempt: u32,
        max_attempts: u32,
        cooldown_ticks: u32,
    },
    StateRestoreValidationCorrectionRetriesExhausted {
        attempts: u32,
        max_attempts: u32,
    },
    StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
        consecutive_mismatch_cycles: u32,
        disable_after_mismatch_cycles: u32,
    },
    StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
        remaining_reconnect_cycles_after_this_cycle: u32,
    },
    StateRestoreValidationCorrectionRecoveryCooldownReenabled,
    RestoringPlaylist,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ControllerAuthTransitionNotification {
    Attempting {
        room: String,
    },
    Succeeded {
        username: String,
        room: String,
        hide_from_osd: bool,
    },
    Failed {
        username: String,
        room: String,
        hide_from_osd: bool,
    },
}

impl std::fmt::Debug for ControllerAuthTransitionNotification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Attempting { .. } => formatter.write_str("Attempting(<redacted>)"),
            Self::Succeeded { hide_from_osd, .. } => formatter
                .debug_struct("Succeeded")
                .field("identity", &sorotte_secret::REDACTED_SECRET)
                .field("hide_from_osd", hide_from_osd)
                .finish(),
            Self::Failed { hide_from_osd, .. } => formatter
                .debug_struct("Failed")
                .field("identity", &sorotte_secret::REDACTED_SECRET)
                .field("hide_from_osd", hide_from_osd)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ControlledRoomCreationNotification {
    Created { room: String, password: SecretValue },
}

impl std::fmt::Debug for ControlledRoomCreationNotification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created { password, .. } => formatter
                .debug_struct("Created")
                .field("room", &sorotte_secret::REDACTED_SECRET)
                .field("password", password)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum UserChangeNotification {
    Joined {
        username: String,
        room: String,
        hide_from_osd: bool,
    },
    Playing {
        username: String,
        room: String,
        file_name: Option<String>,
        file_duration: Option<f64>,
        include_room_addendum: bool,
        hide_from_osd: bool,
    },
    Left {
        username: String,
        hide_from_osd: bool,
    },
}

impl std::fmt::Debug for UserChangeNotification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Joined { hide_from_osd, .. } => formatter
                .debug_struct("Joined")
                .field("identity", &sorotte_secret::REDACTED_SECRET)
                .field("hide_from_osd", hide_from_osd)
                .finish(),
            Self::Playing {
                file_name,
                file_duration,
                include_room_addendum,
                hide_from_osd,
                ..
            } => formatter
                .debug_struct("Playing")
                .field("identity", &sorotte_secret::REDACTED_SECRET)
                .field(
                    "file_name",
                    &file_name.as_ref().map(|_| sorotte_secret::REDACTED_SECRET),
                )
                .field("file_duration", file_duration)
                .field("include_room_addendum", include_room_addendum)
                .field("hide_from_osd", hide_from_osd)
                .finish(),
            Self::Left { hide_from_osd, .. } => formatter
                .debug_struct("Left")
                .field("username", &sorotte_secret::REDACTED_SECRET)
                .field("hide_from_osd", hide_from_osd)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ChatNotification {
    Message {
        username: Option<String>,
        message: String,
    },
}

impl std::fmt::Debug for ChatNotification {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message { username, message } => formatter
                .debug_struct("Message")
                .field("has_username", &username.is_some())
                .field("message_bytes", &message.len())
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileDifferenceSummary {
    pub filename: bool,
    pub filesize: bool,
    pub fileduration: bool,
}

impl FileDifferenceSummary {
    pub fn has_differences(&self) -> bool {
        self.filename || self.filesize || self.fileduration
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ReconnectPlaylistRestoreIntent {
    pub files: Vec<String>,
    pub index: Option<i64>,
}

impl std::fmt::Debug for ReconnectPlaylistRestoreIntent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReconnectPlaylistRestoreIntent")
            .field("files_count", &self.files.len())
            .field("index", &self.index)
            .finish()
    }
}
