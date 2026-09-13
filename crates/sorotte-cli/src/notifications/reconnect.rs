use super::*;

#[cfg(test)]
pub(crate) fn reconnect_transition_notification_message(
    notification: &ReconnectTransitionNotification,
) -> String {
    shared_reconnect_transition_notification_message(notification)
}

pub(crate) fn reconnect_transition_notification_message_localized(
    notification: &ReconnectTransitionNotification,
    language: Option<&str>,
) -> String {
    shared_reconnect_transition_notification_message_localized(notification, language)
}

pub(crate) fn emit_reconnect_transition_notification(
    notification: &ReconnectTransitionNotification,
) -> anyhow::Result<()> {
    let language = current_runtime_language_tag();
    println!(
        "{}",
        reconnect_transition_notification_message_localized(notification, language.as_deref(),)
    );
    Ok(())
}

fn emit_reconnect_transition_notification_to_player(
    player: &mut MpvAdapter,
    notification: &ReconnectTransitionNotification,
) {
    let language = current_runtime_language_tag();
    let message =
        reconnect_transition_notification_message_localized(notification, language.as_deref());
    emit_sorotte_player_osd_notification(
        player,
        &message,
        SyncplayOsdKind::Notification,
        "reconnect notification",
    );
}

pub(crate) fn flush_reconnect_notifications<F>(
    runtime: &mut ClientApplication<MpvAdapter>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ReconnectTransitionNotification) -> anyhow::Result<()>,
{
    while let Some(notification) = runtime.pending_reconnect_notification().cloned() {
        runtime.with_player_io(|player| {
            emit_reconnect_transition_notification_to_player(player, &notification);
        });
        notify(&notification)?;
        let acknowledged = runtime.acknowledge_reconnect_notification();
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}
