use super::*;

#[cfg(test)]
pub(crate) fn format_duration_legacy(time_seconds: f64) -> String {
    shared_format_duration_legacy(time_seconds)
}

#[cfg(test)]
pub(crate) fn user_change_notification_message(notification: &UserChangeNotification) -> String {
    shared_user_change_notification_message(notification)
}

pub(crate) fn user_change_notification_message_localized_legacy_compatible(
    notification: &UserChangeNotification,
    language: Option<&str>,
) -> String {
    shared_user_change_notification_message_localized_legacy_compatible(notification, language)
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
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    println!(
        "{}",
        user_change_notification_message_localized_legacy_compatible(
            notification,
            language.as_deref(),
        )
    );
    Ok(())
}

fn emit_user_change_notification_to_player_legacy_compatible(
    player: &mut MpvAdapter,
    notification: &UserChangeNotification,
) {
    if user_change_notification_hidden_from_osd(notification) {
        return;
    }

    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let message = user_change_notification_message_localized_legacy_compatible(
        notification,
        language.as_deref(),
    );
    emit_syncplay_player_osd_notification_legacy_compatible(
        player,
        &message,
        LegacySyncplayOsdKind::Notification,
        "user-change notification",
    );
}

pub(crate) fn flush_user_change_notifications_legacy_compatible(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
) -> anyhow::Result<()> {
    for notification in runtime.drain_user_change_notifications() {
        emit_user_change_notification_to_player_legacy_compatible(
            runtime.player_mut(),
            &notification,
        );
        emit_user_change_notification(&notification)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_user_change_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&UserChangeNotification) -> anyhow::Result<()>,
{
    runtime.drain_user_change_notifications_to_sink(|notification| notify(notification))
}
