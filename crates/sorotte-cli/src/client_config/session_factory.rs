use super::overrides::{
    behavior_overrides_from_env, chat_policy_overrides_from_env, readiness_overrides_from_env,
};
use super::*;

#[cfg(test)]
pub(crate) fn create_client_runtime(config: &ClientLoopConfig) -> ClientApplication<MpvAdapter> {
    let session = create_client_session(config);
    let mut player = SimulatedPlayer::new().into_inner();
    apply_legacy_syncplay_ui_settings_to_mpv_adapter_legacy_compatible(&mut player, None)
        .expect("default legacy mpv OSD/chat settings should apply");
    ClientApplication::new(session, player)
}

pub(crate) fn create_client_session(config: &ClientLoopConfig) -> ClientSession {
    let mut session = ClientSession::default();
    session.set_autoplay_enabled(config.autoplay_enabled);
    if let Some(control_password) = config.controlled_room_password_override.as_deref() {
        session.remember_control_password_for_room(&config.room, control_password);
    }
    if let Some(show_same_room_osd) = config.show_same_room_osd_override {
        session.behavior_config_mut().show_same_room_osd = show_same_room_osd;
    }
    if let Some(show_osd_warnings) = config.show_osd_warnings_override {
        session.behavior_config_mut().show_osd_warnings = show_osd_warnings;
    }
    if let Some(show_noncontroller_osd) = config.show_noncontroller_osd_override {
        session.behavior_config_mut().show_noncontroller_osd = show_noncontroller_osd;
    }
    if let Some(show_different_room_osd) = config.show_different_room_osd_override {
        session.behavior_config_mut().show_different_room_osd = show_different_room_osd;
    }
    if let Some(pause_on_leave) = config.pause_on_leave_override {
        session.behavior_config_mut().pause_on_leave = pause_on_leave;
    }
    if let Some(loop_at_end_of_playlist) = config.loop_at_end_of_playlist_override {
        session.behavior_config_mut().loop_at_end_of_playlist = loop_at_end_of_playlist;
    }
    if let Some(loop_single_files) = config.loop_single_files_override {
        session.behavior_config_mut().loop_single_files = loop_single_files;
    }
    if let Some(only_switch_to_trusted_domains) = config.only_switch_to_trusted_domains_override {
        session.behavior_config_mut().only_switch_to_trusted_domains =
            only_switch_to_trusted_domains;
    }
    if let Some(trusted_domains) = config.trusted_domains_override.as_ref() {
        session.behavior_config_mut().trusted_domains = trusted_domains.clone();
    }
    if let Some(rewind_on_desync) = config.rewind_on_desync_override {
        session.desync_config_mut().rewind_on_desync = rewind_on_desync;
    }
    if let Some(fastforward_on_desync) = config.fastforward_on_desync_override {
        session.desync_config_mut().fastforward_on_desync = fastforward_on_desync;
    }
    if let Some(slow_on_desync) = config.slow_on_desync_override {
        session.desync_config_mut().slow_on_desync = slow_on_desync;
    }
    if let Some(rewind_threshold_seconds) = config.rewind_threshold_seconds_override {
        session.desync_config_mut().rewind_threshold_seconds = rewind_threshold_seconds;
    }
    if let Some(fastforward_threshold_seconds) = config.fastforward_threshold_seconds_override {
        session.desync_config_mut().fastforward_threshold_seconds = fastforward_threshold_seconds;
    }
    if let Some(slowdown_threshold_seconds) = config.slowdown_threshold_seconds_override {
        session.desync_config_mut().slowdown_threshold_seconds = slowdown_threshold_seconds;
    }
    apply_client_behavior_overrides(&mut session, &behavior_overrides_from_env());
    {
        let readiness_config = session.readiness_autoplay_config_mut();
        readiness_config.autoplay_require_same_filenames = config.autoplay_require_same_filenames;
        if let Some(unpause_action) = config.unpause_action_override.as_ref() {
            readiness_config.unpause_action = unpause_action.clone();
        }
        if let Some(auto_play_threshold) = config.auto_play_threshold_override.as_ref() {
            readiness_config.auto_play_threshold = match auto_play_threshold {
                AutoplayThresholdOverride::Disable => None,
                AutoplayThresholdOverride::Set(value) => Some(*value),
            };
        }
        if let Some(show_duration_notification) = config.show_duration_notification_override {
            readiness_config.show_duration_notification = show_duration_notification;
        }
        if let Some(different_duration_threshold_seconds) =
            config.different_duration_threshold_seconds_override
        {
            readiness_config.different_duration_threshold_seconds =
                different_duration_threshold_seconds;
        }
        apply_readiness_autoplay_overrides(readiness_config, &readiness_overrides_from_env());
    }
    apply_chat_policy_overrides(&mut session, &chat_policy_overrides_from_env());
    session.reconnect_policy_mut().max_retries = config.max_retries;
    session
}
