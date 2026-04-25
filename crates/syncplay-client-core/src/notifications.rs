use super::*;

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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlledRoomCreationNotification {
    Created { room: String, password: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
        file_duration: Option<Value>,
        include_room_addendum: bool,
        hide_from_osd: bool,
    },
    Left {
        username: String,
        hide_from_osd: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatNotification {
    Message {
        username: Option<String>,
        message: String,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectPlaylistRestoreIntent {
    pub files: Vec<String>,
    pub index: Option<i64>,
}
