use crate::{
    runtime_config::{
        ClientConfig, RoomBufferingPolicy, StartSynchronizationPolicy, StartTimeoutAction,
        StreamingPlaybackConfig,
    },
    stored_settings::StoredClientSettings,
};

/// A settings shortcut, not room authority or a separately persisted identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronizationPreset {
    Standard,
    WatchTogether,
}

/// The complete ownership boundary used by application, validation and GUI edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronizationPresetField {
    StartPolicy,
    StartQuorum,
    StartTimeout,
    StartTimeoutAction,
    RoomBufferingPolicy,
    RoomQuorum,
    RoomMaximumPause,
}

impl SynchronizationPresetField {
    pub const ALL: [Self; 7] = [
        Self::StartPolicy,
        Self::StartQuorum,
        Self::StartTimeout,
        Self::StartTimeoutAction,
        Self::RoomBufferingPolicy,
        Self::RoomQuorum,
        Self::RoomMaximumPause,
    ];

    fn stored_field_name(self) -> &'static str {
        match self {
            Self::StartPolicy => "streaming_start_policy",
            Self::StartQuorum => "streaming_start_quorum_percent",
            Self::StartTimeout => "streaming_start_timeout_seconds",
            Self::StartTimeoutAction => "streaming_start_timeout_action",
            Self::RoomBufferingPolicy => "streaming_room_buffering_policy",
            Self::RoomQuorum => "streaming_room_quorum_percent",
            Self::RoomMaximumPause => "streaming_room_max_pause_seconds",
        }
    }

    fn apply(self, config: &StreamingPlaybackConfig, settings: &mut StoredClientSettings) {
        let start = &config.start_synchronization;
        let room = &config.room_buffering;
        match self {
            Self::StartPolicy => {
                settings.streaming_start_policy = Some(start.policy.config_value().to_owned());
            }
            Self::StartQuorum => settings.streaming_start_quorum_percent = Some(start.quorum.get()),
            Self::StartTimeout => {
                settings.streaming_start_timeout_seconds = Some(start.timeout.get())
            }
            Self::StartTimeoutAction => {
                settings.streaming_start_timeout_action =
                    Some(start.timeout_action.config_value().to_owned());
            }
            Self::RoomBufferingPolicy => {
                settings.streaming_room_buffering_policy =
                    Some(room.policy.config_value().to_owned());
            }
            Self::RoomQuorum => settings.streaming_room_quorum_percent = Some(room.quorum.get()),
            Self::RoomMaximumPause => {
                settings.streaming_room_max_pause_seconds = Some(room.maximum_pause.get())
            }
        }
    }
}

impl SynchronizationPreset {
    pub const ALL: [Self; 2] = [Self::Standard, Self::WatchTogether];

    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::WatchTogether => "watch-together",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::WatchTogether => "Watch together",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Standard => "Start immediately and let each participant buffer independently.",
            Self::WatchTogether => {
                "Wait for every required participant to be ready and able to play. Ask the controller after 30 seconds. Pause together for buffering, for up to 30 seconds."
            }
        }
    }

    fn configuration(self) -> StreamingPlaybackConfig {
        let mut config = StreamingPlaybackConfig::default();
        if self == Self::WatchTogether {
            config.start_synchronization.policy = StartSynchronizationPolicy::WaitForAllEligible;
            config.start_synchronization.quorum =
                crate::runtime_config::Percent::new(100.0).expect("the all-member quorum is valid");
            config.start_synchronization.timeout = crate::runtime_config::Seconds::new(30.0)
                .expect("the bounded start timeout is valid");
            config.start_synchronization.timeout_action = StartTimeoutAction::AskController;
            config.room_buffering.policy = RoomBufferingPolicy::PauseEligible;
            config.room_buffering.quorum = config.start_synchronization.quorum;
            config.room_buffering.maximum_pause = config.start_synchronization.timeout;
        }
        config
    }

    pub fn apply_to(self, settings: &mut StoredClientSettings) {
        let config = self.configuration();
        for field in SynchronizationPresetField::ALL {
            field.apply(&config, settings);
        }
    }
}

pub fn detect_synchronization_preset(
    settings: &StoredClientSettings,
) -> Option<SynchronizationPreset> {
    let resolution = ClientConfig::resolve(settings);
    if resolution.issues.iter().any(|issue| {
        SynchronizationPresetField::ALL
            .iter()
            .any(|field| field.stored_field_name() == issue.field)
    }) {
        return None;
    }
    let actual = &resolution.config.playback.streaming;
    SynchronizationPreset::ALL.into_iter().find(|preset| {
        let expected = preset.configuration();
        actual.start_synchronization == expected.start_synchronization
            && actual.room_buffering == expected.room_buffering
    })
}

#[cfg(test)]
mod tests;
