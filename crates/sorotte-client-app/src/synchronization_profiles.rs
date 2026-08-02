use crate::{
    legacy_settings::StoredClientSettingsV1,
    runtime_config::{
        ClientConfig, RoomBufferingPolicy, StartSynchronizationPolicy, StartTimeoutAction,
        StreamingRecoveryPolicy,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SynchronizationProfileId {
    PrivateRoom,
    LargeControlledRoom,
    PublicRoom,
}

impl SynchronizationProfileId {
    pub const ALL: &'static [Self] = &[
        Self::PrivateRoom,
        Self::LargeControlledRoom,
        Self::PublicRoom,
    ];

    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::PrivateRoom => "private-room",
            Self::LargeControlledRoom => "large-controlled-room",
            Self::PublicRoom => "public-room",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::PrivateRoom => "Private Room",
            Self::LargeControlledRoom => "Large Controlled Room",
            Self::PublicRoom => "Public Room",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::PrivateRoom => {
                "Wait for every capable participant and pause for any eligible member."
            }
            Self::LargeControlledRoom => {
                "Use a 75% quorum so a larger controlled room can keep moving."
            }
            Self::PublicRoom => {
                "Start immediately and let each participant manage buffering independently."
            }
        }
    }

    pub const fn definition(self) -> SynchronizationProfile {
        match self {
            Self::PrivateRoom => SynchronizationProfile {
                id: self,
                buffer_target_seconds: 120.0,
                read_ahead_seconds: 7_200.0,
                memory_cache_mebibytes: 256,
                disk_cache_enabled: true,
                recovery_policy: StreamingRecoveryPolicy::Balanced,
                maximum_catchup_rate: 1.05,
                hard_seek_threshold_seconds: 8.0,
                maximum_hard_seeks_per_episode: 1,
                stability_interval_seconds: 4.0,
                recovery_retry_budget: 1,
                recovery_cooldown_seconds: 10.0,
                room_buffering_policy: RoomBufferingPolicy::PauseEligible,
                room_quorum_percent: 100.0,
                room_maximum_pause_seconds: 180.0,
                start_policy: StartSynchronizationPolicy::WaitForAllEligible,
                start_quorum_percent: 100.0,
                start_timeout_seconds: 180.0,
                start_timeout_action: StartTimeoutAction::Continue,
                rewind_on_desync: true,
                fastforward_on_desync: true,
                slow_on_desync: true,
                dont_slow_down_with_me: false,
                rewind_threshold_seconds: 4.0,
                fastforward_threshold_seconds: 3.0,
                slowdown_threshold_seconds: 0.75,
            },
            Self::LargeControlledRoom => SynchronizationProfile {
                id: self,
                buffer_target_seconds: 60.0,
                read_ahead_seconds: 7_200.0,
                memory_cache_mebibytes: 256,
                disk_cache_enabled: true,
                recovery_policy: StreamingRecoveryPolicy::Balanced,
                maximum_catchup_rate: 1.05,
                hard_seek_threshold_seconds: 8.0,
                maximum_hard_seeks_per_episode: 1,
                stability_interval_seconds: 4.0,
                recovery_retry_budget: 1,
                recovery_cooldown_seconds: 10.0,
                room_buffering_policy: RoomBufferingPolicy::Quorum,
                room_quorum_percent: 75.0,
                room_maximum_pause_seconds: 90.0,
                start_policy: StartSynchronizationPolicy::Quorum,
                start_quorum_percent: 75.0,
                start_timeout_seconds: 90.0,
                start_timeout_action: StartTimeoutAction::Continue,
                rewind_on_desync: true,
                fastforward_on_desync: true,
                slow_on_desync: true,
                dont_slow_down_with_me: false,
                rewind_threshold_seconds: 4.0,
                fastforward_threshold_seconds: 5.0,
                slowdown_threshold_seconds: 0.75,
            },
            Self::PublicRoom => SynchronizationProfile {
                id: self,
                buffer_target_seconds: 60.0,
                read_ahead_seconds: 7_200.0,
                memory_cache_mebibytes: 256,
                disk_cache_enabled: true,
                recovery_policy: StreamingRecoveryPolicy::Balanced,
                maximum_catchup_rate: 1.05,
                hard_seek_threshold_seconds: 8.0,
                maximum_hard_seeks_per_episode: 1,
                stability_interval_seconds: 4.0,
                recovery_retry_budget: 1,
                recovery_cooldown_seconds: 10.0,
                room_buffering_policy: RoomBufferingPolicy::Independent,
                room_quorum_percent: 75.0,
                room_maximum_pause_seconds: 30.0,
                start_policy: StartSynchronizationPolicy::Immediate,
                start_quorum_percent: 75.0,
                start_timeout_seconds: 15.0,
                start_timeout_action: StartTimeoutAction::Continue,
                rewind_on_desync: true,
                fastforward_on_desync: true,
                slow_on_desync: true,
                dont_slow_down_with_me: false,
                rewind_threshold_seconds: 4.0,
                fastforward_threshold_seconds: 5.0,
                slowdown_threshold_seconds: 0.75,
            },
        }
    }

    pub fn apply_to(self, settings: &mut StoredClientSettingsV1) {
        self.definition().apply_to(settings);
    }

    pub fn matches(self, settings: &StoredClientSettingsV1) -> bool {
        self.definition().matches(settings)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynchronizationProfile {
    pub id: SynchronizationProfileId,
    pub buffer_target_seconds: f64,
    pub read_ahead_seconds: f64,
    pub memory_cache_mebibytes: u64,
    pub disk_cache_enabled: bool,
    pub recovery_policy: StreamingRecoveryPolicy,
    pub maximum_catchup_rate: f64,
    pub hard_seek_threshold_seconds: f64,
    pub maximum_hard_seeks_per_episode: u32,
    pub stability_interval_seconds: f64,
    pub recovery_retry_budget: u32,
    pub recovery_cooldown_seconds: f64,
    pub room_buffering_policy: RoomBufferingPolicy,
    pub room_quorum_percent: f64,
    pub room_maximum_pause_seconds: f64,
    pub start_policy: StartSynchronizationPolicy,
    pub start_quorum_percent: f64,
    pub start_timeout_seconds: f64,
    pub start_timeout_action: StartTimeoutAction,
    pub rewind_on_desync: bool,
    pub fastforward_on_desync: bool,
    pub slow_on_desync: bool,
    pub dont_slow_down_with_me: bool,
    pub rewind_threshold_seconds: f64,
    pub fastforward_threshold_seconds: f64,
    pub slowdown_threshold_seconds: f64,
}

impl SynchronizationProfile {
    pub fn apply_to(self, settings: &mut StoredClientSettingsV1) {
        settings.streaming_buffer_target_seconds = Some(self.buffer_target_seconds);
        settings.streaming_read_ahead_seconds = Some(self.read_ahead_seconds);
        settings.streaming_memory_cache_mebibytes = Some(self.memory_cache_mebibytes);
        settings.streaming_disk_cache_enabled = Some(self.disk_cache_enabled);
        settings.streaming_recovery_policy = Some(self.recovery_policy.config_value().to_owned());
        settings.streaming_max_catchup_rate = Some(self.maximum_catchup_rate);
        settings.streaming_hard_seek_threshold_seconds = Some(self.hard_seek_threshold_seconds);
        settings.streaming_max_hard_seeks_per_episode = Some(self.maximum_hard_seeks_per_episode);
        settings.streaming_stability_interval_seconds = Some(self.stability_interval_seconds);
        settings.streaming_recovery_retry_budget = Some(self.recovery_retry_budget);
        settings.streaming_recovery_cooldown_seconds = Some(self.recovery_cooldown_seconds);
        settings.streaming_room_buffering_policy =
            Some(self.room_buffering_policy.config_value().to_owned());
        settings.streaming_room_quorum_percent = Some(self.room_quorum_percent);
        settings.streaming_room_max_pause_seconds = Some(self.room_maximum_pause_seconds);
        settings.streaming_start_policy = Some(self.start_policy.config_value().to_owned());
        settings.streaming_start_quorum_percent = Some(self.start_quorum_percent);
        settings.streaming_start_timeout_seconds = Some(self.start_timeout_seconds);
        settings.streaming_start_timeout_action =
            Some(self.start_timeout_action.config_value().to_owned());
        settings.rewind_on_desync = Some(self.rewind_on_desync);
        settings.fastforward_on_desync = Some(self.fastforward_on_desync);
        settings.slow_on_desync = Some(self.slow_on_desync);
        settings.dont_slow_down_with_me = Some(self.dont_slow_down_with_me);
        settings.rewind_threshold_seconds = Some(self.rewind_threshold_seconds);
        settings.fastforward_threshold_seconds = Some(self.fastforward_threshold_seconds);
        settings.slowdown_threshold_seconds = Some(self.slowdown_threshold_seconds);
    }

    pub fn matches(self, settings: &StoredClientSettingsV1) -> bool {
        let config = ClientConfig::resolve(settings).config;
        let streaming = &config.playback.streaming;
        let sync = &config.synchronization;

        approximately_equal(streaming.buffering.target.get(), self.buffer_target_seconds)
            && approximately_equal(
                streaming.buffering.read_ahead.get(),
                self.read_ahead_seconds,
            )
            && streaming.buffering.memory_cache_mebibytes == self.memory_cache_mebibytes
            && streaming.buffering.disk_cache_enabled == self.disk_cache_enabled
            && streaming.recovery.policy == self.recovery_policy
            && approximately_equal(
                streaming.recovery.max_catchup_rate.get(),
                self.maximum_catchup_rate,
            )
            && approximately_equal(
                streaming.recovery.hard_seek_threshold.get(),
                self.hard_seek_threshold_seconds,
            )
            && streaming.recovery.max_hard_seeks_per_episode == self.maximum_hard_seeks_per_episode
            && approximately_equal(
                streaming.recovery.stability_interval.get(),
                self.stability_interval_seconds,
            )
            && streaming.recovery.retry_budget == self.recovery_retry_budget
            && approximately_equal(
                streaming.recovery.cooldown.get(),
                self.recovery_cooldown_seconds,
            )
            && streaming.room_buffering.policy == self.room_buffering_policy
            && approximately_equal(
                streaming.room_buffering.quorum.get(),
                self.room_quorum_percent,
            )
            && approximately_equal(
                streaming.room_buffering.maximum_pause.get(),
                self.room_maximum_pause_seconds,
            )
            && streaming.start_synchronization.policy == self.start_policy
            && approximately_equal(
                streaming.start_synchronization.quorum.get(),
                self.start_quorum_percent,
            )
            && approximately_equal(
                streaming.start_synchronization.timeout.get(),
                self.start_timeout_seconds,
            )
            && streaming.start_synchronization.timeout_action == self.start_timeout_action
            && sync.rewind_on_desync == self.rewind_on_desync
            && sync.fastforward_on_desync == self.fastforward_on_desync
            && sync.slow_on_desync == self.slow_on_desync
            && sync.dont_slow_down_with_me == self.dont_slow_down_with_me
            && approximately_equal(sync.rewind_threshold.get(), self.rewind_threshold_seconds)
            && approximately_equal(
                sync.fastforward_threshold.get(),
                self.fastforward_threshold_seconds,
            )
            && approximately_equal(
                sync.slowdown_threshold.get(),
                self.slowdown_threshold_seconds,
            )
    }
}

pub fn detect_synchronization_profile(
    settings: &StoredClientSettingsV1,
) -> Option<SynchronizationProfileId> {
    SynchronizationProfileId::ALL
        .iter()
        .copied()
        .find(|profile| profile.matches(settings))
}

fn approximately_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= f64::EPSILON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_settings_resolve_to_the_public_room_profile() {
        assert_eq!(
            detect_synchronization_profile(&StoredClientSettingsV1::default()),
            Some(SynchronizationProfileId::PublicRoom)
        );
    }

    #[test]
    fn applying_each_profile_is_exact_idempotent_and_preserves_unrelated_settings() {
        for &profile in SynchronizationProfileId::ALL {
            let mut settings = StoredClientSettingsV1 {
                host: Some("sync.example".to_owned()),
                room: Some("lounge".to_owned()),
                server_password: Some("secret".into()),
                streaming_quality_preset: Some("720p".to_owned()),
                ..StoredClientSettingsV1::default()
            };

            profile.apply_to(&mut settings);
            let applied_once = settings.clone();
            profile.apply_to(&mut settings);

            assert_eq!(settings, applied_once);
            assert_eq!(detect_synchronization_profile(&settings), Some(profile));
            assert_eq!(settings.host.as_deref(), Some("sync.example"));
            assert_eq!(settings.room.as_deref(), Some("lounge"));
            assert_eq!(
                settings
                    .server_password
                    .as_ref()
                    .map(|password| password.expose_secret()),
                Some("secret")
            );
            assert_eq!(settings.streaming_quality_preset.as_deref(), Some("720p"));
        }
    }

    #[test]
    fn profiles_resolve_to_the_documented_sync_and_episode_cache_policies() {
        let cases = [
            (
                SynchronizationProfileId::PrivateRoom,
                120.0,
                RoomBufferingPolicy::PauseEligible,
                100.0,
                180.0,
                StartSynchronizationPolicy::WaitForAllEligible,
                100.0,
                180.0,
                3.0,
            ),
            (
                SynchronizationProfileId::LargeControlledRoom,
                60.0,
                RoomBufferingPolicy::Quorum,
                75.0,
                90.0,
                StartSynchronizationPolicy::Quorum,
                75.0,
                90.0,
                5.0,
            ),
            (
                SynchronizationProfileId::PublicRoom,
                60.0,
                RoomBufferingPolicy::Independent,
                75.0,
                30.0,
                StartSynchronizationPolicy::Immediate,
                75.0,
                15.0,
                5.0,
            ),
        ];

        for (
            profile,
            buffer_target,
            room_policy,
            room_quorum,
            maximum_pause,
            start_policy,
            start_quorum,
            start_timeout,
            fastforward_threshold,
        ) in cases
        {
            let mut settings = StoredClientSettingsV1::default();
            profile.apply_to(&mut settings);
            let config =
                ClientConfig::try_from_stored(&settings).expect("built-in profile should resolve");
            let synchronization = &config.synchronization;
            let streaming = config.playback.streaming;

            assert_eq!(streaming.buffering.target.get(), buffer_target);
            assert_eq!(streaming.buffering.read_ahead.get(), 7_200.0);
            assert_eq!(streaming.buffering.memory_cache_mebibytes, 256);
            assert!(streaming.buffering.disk_cache_enabled);
            assert_eq!(streaming.recovery.policy, StreamingRecoveryPolicy::Balanced);
            assert_eq!(streaming.recovery.max_catchup_rate.get(), 1.05);
            assert_eq!(streaming.recovery.hard_seek_threshold.get(), 8.0);
            assert_eq!(streaming.recovery.max_hard_seeks_per_episode, 1);
            assert_eq!(streaming.recovery.stability_interval.get(), 4.0);
            assert_eq!(streaming.recovery.retry_budget, 1);
            assert_eq!(streaming.recovery.cooldown.get(), 10.0);
            assert_eq!(streaming.room_buffering.policy, room_policy);
            assert_eq!(streaming.room_buffering.quorum.get(), room_quorum);
            assert_eq!(streaming.room_buffering.maximum_pause.get(), maximum_pause);
            assert_eq!(streaming.start_synchronization.policy, start_policy);
            assert_eq!(streaming.start_synchronization.quorum.get(), start_quorum);
            assert_eq!(streaming.start_synchronization.timeout.get(), start_timeout);
            assert_eq!(
                streaming.start_synchronization.timeout_action,
                StartTimeoutAction::Continue
            );
            assert!(synchronization.rewind_on_desync);
            assert!(synchronization.fastforward_on_desync);
            assert!(synchronization.slow_on_desync);
            assert!(!synchronization.dont_slow_down_with_me);
            assert_eq!(synchronization.rewind_threshold.get(), 4.0);
            assert_eq!(
                synchronization.fastforward_threshold.get(),
                fastforward_threshold
            );
            assert_eq!(synchronization.slowdown_threshold.get(), 0.75);
        }
    }

    #[test]
    fn manual_owned_field_change_is_detected_as_custom() {
        let mut settings = StoredClientSettingsV1::default();
        SynchronizationProfileId::PrivateRoom.apply_to(&mut settings);
        settings.streaming_read_ahead_seconds = Some(3_600.0);

        assert_eq!(detect_synchronization_profile(&settings), None);
    }

    #[test]
    fn public_profile_user_facing_copy_has_no_compatibility_framing() {
        let profile = SynchronizationProfileId::PublicRoom;
        assert_eq!(profile.label(), "Public Room");
        assert!(!profile.label().to_ascii_lowercase().contains("compat"));
        assert!(
            !profile
                .description()
                .to_ascii_lowercase()
                .contains("compat")
        );
    }
}
