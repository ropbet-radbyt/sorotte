mod autoplay_rooms;
mod public_server_media;
mod reconnect_state;
mod update_controller_access;
mod user_status;

pub(super) fn localize_pattern_message(message: &str, language: Option<&str>) -> Option<String> {
    public_server_media::localize_public_server_media_message(message, language)
        .or_else(|| user_status::localize_user_status_message(message, language))
        .or_else(|| reconnect_state::localize_reconnect_state_message(message, language))
        .or_else(|| autoplay_rooms::localize_autoplay_rooms_message(message, language))
        .or_else(|| {
            update_controller_access::localize_update_controller_access_message(message, language)
        })
}
