use super::*;

#[cfg(test)]
pub(crate) fn format_duration(time_seconds: f64) -> String {
    shared_format_duration(time_seconds)
}

#[cfg(test)]
pub(crate) fn user_change_notification_message(notification: &UserChangeNotification) -> String {
    shared_user_change_notification_message(notification)
}

pub(crate) fn user_change_notification_message_localized(
    notification: &UserChangeNotification,
    language: Option<&str>,
) -> String {
    shared_user_change_notification_message_localized(notification, language)
}

pub(crate) fn user_change_notification_hidden_from_osd(
    notification: &UserChangeNotification,
) -> bool {
    shared_user_change_notification_hidden_from_osd(notification)
}

fn emit_user_change_notification(notification: &UserChangeNotification) -> anyhow::Result<()> {
    if user_change_notification_hidden_from_osd(notification) {
        return Ok(());
    }
    let language = current_runtime_language_tag();
    println!(
        "{}",
        user_change_notification_message_localized(notification, language.as_deref(),)
    );
    Ok(())
}

fn emit_user_change_notification_to_player(
    player: &mut MpvAdapter,
    notification: &UserChangeNotification,
) {
    if user_change_notification_hidden_from_osd(notification) {
        return;
    }

    let language = current_runtime_language_tag();
    let message = user_change_notification_message_localized(notification, language.as_deref());
    emit_sorotte_player_osd_notification(
        player,
        &message,
        SyncplayOsdKind::Notification,
        "user-change notification",
    );
}

pub(crate) fn flush_user_change_notifications(
    runtime: &mut ClientApplication<MpvAdapter>,
) -> anyhow::Result<()> {
    while let Some(notification) = runtime.pending_user_change_notification().cloned() {
        runtime.with_player_io(|player| {
            emit_user_change_notification_to_player(player, &notification);
        });
        emit_user_change_notification(&notification)?;
        let acknowledged = runtime.acknowledge_user_change_notification();
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_user_change_notifications_to_sink<F>(
    runtime: &mut ClientApplication<MpvAdapter>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&UserChangeNotification) -> anyhow::Result<()>,
{
    runtime.drain_user_change_notifications_to_sink(|notification| notify(notification))
}
