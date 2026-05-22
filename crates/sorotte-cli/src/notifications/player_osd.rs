use super::*;

pub(crate) fn emit_mpv_osd_error_warning_legacy_compatible(context: &str, error: PlayerError) {
    eprintln!("warning: failed to display {context} via mpv OSD: {error}");
}

pub(crate) fn emit_sorotte_player_osd_notification_legacy_compatible(
    player: &mut MpvAdapter,
    message: &str,
    kind: LegacySyncplayOsdKind,
    context: &str,
) {
    if let Err(error) = player.show_syncplay_legacy_message(message, kind) {
        emit_mpv_osd_error_warning_legacy_compatible(context, error);
    }
}

pub(crate) fn emit_sorotte_player_chat_notification_legacy_compatible(
    player: &mut MpvAdapter,
    message: &str,
) {
    if let Err(error) = player.show_syncplay_legacy_chat_message(message) {
        emit_mpv_osd_error_warning_legacy_compatible("chat notification", error);
    }
}
