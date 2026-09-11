use super::*;

pub(crate) fn emit_mpv_osd_error_warning(context: &str, error: PlayerError) {
    eprintln!("warning: failed to display {context} via mpv OSD: {error}");
}

pub(crate) fn emit_sorotte_player_osd_notification(
    player: &mut MpvAdapter,
    message: &str,
    kind: SyncplayOsdKind,
    context: &str,
) {
    if let Err(error) = player.show_syncplay_message(message, kind) {
        emit_mpv_osd_error_warning(context, error);
    }
}

pub(crate) fn emit_sorotte_player_chat_notification(player: &mut MpvAdapter, message: &str) {
    if let Err(error) = player.show_syncplay_chat_message(message) {
        emit_mpv_osd_error_warning("chat notification", error);
    }
}
