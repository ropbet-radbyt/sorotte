use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::app) struct GuiSessionRoomPlaystate {
    pub(in crate::app) position_seconds: Option<f64>,
    pub(in crate::app) paused: Option<bool>,
    pub(in crate::app) do_seek: Option<bool>,
    pub(in crate::app) set_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum GuiLocalPlayerUnpauseDecision {
    NotApplicable,
    Allow,
    Block,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiAttachedPlayerRuntimeAction {
    Paused {
        paused: bool,
        cause: PlayerCommandCause,
    },
    Position(f64),
    PlaybackRate(f64),
    DesyncPlaybackRate {
        playback_rate: f64,
        rollback: DesyncCorrectionDispatchSnapshot,
    },
    Coordinator {
        command_id: CoordinatorCommandId,
        command: CoordinatorPlayerCommand,
    },
}
