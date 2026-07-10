use super::*;

#[cfg(test)]
pub(crate) fn reconnect_transition_notification_message(
    notification: &ReconnectTransitionNotification,
) -> String {
    shared_reconnect_transition_notification_message(notification)
}

pub(crate) fn reconnect_transition_notification_message_localized_legacy_compatible(
    notification: &ReconnectTransitionNotification,
    language: Option<&str>,
) -> String {
    shared_reconnect_transition_notification_message_localized_legacy_compatible(
        notification,
        language,
    )
}

fn emit_reconnect_transition_notification(
    notification: &ReconnectTransitionNotification,
) -> anyhow::Result<()> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    println!(
        "{}",
        reconnect_transition_notification_message_localized_legacy_compatible(
            notification,
            language.as_deref(),
        )
    );
    Ok(())
}

fn emit_reconnect_transition_notification_to_player_legacy_compatible(
    player: &mut MpvAdapter,
    notification: &ReconnectTransitionNotification,
) {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let message = reconnect_transition_notification_message_localized_legacy_compatible(
        notification,
        language.as_deref(),
    );
    emit_sorotte_player_osd_notification_legacy_compatible(
        player,
        &message,
        LegacySyncplayOsdKind::Notification,
        "reconnect notification",
    );
}

pub(crate) fn flush_reconnect_notifications_legacy_compatible(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
) -> anyhow::Result<()> {
    while let Some(notification) = runtime.pending_reconnect_notification().cloned() {
        emit_reconnect_transition_notification_to_player_legacy_compatible(
            runtime.player_mut(),
            &notification,
        );
        emit_reconnect_transition_notification(&notification)?;
        let acknowledged = runtime.acknowledge_reconnect_notification();
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_reconnect_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ReconnectTransitionNotification) -> anyhow::Result<()>,
{
    runtime.drain_reconnect_notifications_to_sink(|notification| notify(notification))
}
