use super::*;

#[cfg(test)]
pub(crate) fn controller_auth_transition_notification_message(
    notification: &ControllerAuthTransitionNotification,
) -> String {
    shared_controller_auth_transition_notification_message(notification)
}

pub(crate) fn controller_auth_transition_notification_message_localized(
    notification: &ControllerAuthTransitionNotification,
    language: Option<&str>,
) -> String {
    shared_controller_auth_transition_notification_message_localized(notification, language)
}

pub(crate) fn controller_auth_notification_hidden_from_osd(
    notification: &ControllerAuthTransitionNotification,
) -> bool {
    shared_controller_auth_notification_hidden_from_osd(notification)
}

fn emit_controller_auth_transition_notification(
    notification: &ControllerAuthTransitionNotification,
) -> anyhow::Result<()> {
    if controller_auth_notification_hidden_from_osd(notification) {
        return Ok(());
    }
    let language = current_runtime_language_tag();
    println!(
        "{}",
        controller_auth_transition_notification_message_localized(
            notification,
            language.as_deref(),
        )
    );
    Ok(())
}

fn emit_controller_auth_transition_notification_to_player(
    player: &mut MpvAdapter,
    notification: &ControllerAuthTransitionNotification,
) {
    if controller_auth_notification_hidden_from_osd(notification) {
        return;
    }

    let language = current_runtime_language_tag();
    let message = controller_auth_transition_notification_message_localized(
        notification,
        language.as_deref(),
    );
    emit_sorotte_player_osd_notification(
        player,
        &message,
        SyncplayOsdKind::Notification,
        "controller-auth notification",
    );
}

pub(crate) fn flush_controller_auth_notifications(
    runtime: &mut ClientApplication<MpvAdapter>,
) -> anyhow::Result<()> {
    while let Some(notification) = runtime.pending_controller_auth_notification().cloned() {
        runtime.with_player_io(|player| {
            emit_controller_auth_transition_notification_to_player(player, &notification);
        });
        emit_controller_auth_transition_notification(&notification)?;
        let acknowledged = runtime.acknowledge_controller_auth_notification();
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_controller_auth_notifications_to_sink<F>(
    runtime: &mut ClientApplication<MpvAdapter>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ControllerAuthTransitionNotification) -> anyhow::Result<()>,
{
    runtime.drain_controller_auth_notifications_to_sink(|notification| notify(notification))
}
