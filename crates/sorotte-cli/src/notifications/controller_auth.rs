use super::*;

#[cfg(test)]
pub(crate) fn controller_auth_transition_notification_message(
    notification: &ControllerAuthTransitionNotification,
) -> String {
    shared_controller_auth_transition_notification_message(notification)
}

pub(crate) fn controller_auth_transition_notification_message_localized_legacy_compatible(
    notification: &ControllerAuthTransitionNotification,
    language: Option<&str>,
) -> String {
    shared_controller_auth_transition_notification_message_localized_legacy_compatible(
        notification,
        language,
    )
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
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    println!(
        "{}",
        controller_auth_transition_notification_message_localized_legacy_compatible(
            notification,
            language.as_deref(),
        )
    );
    Ok(())
}

fn emit_controller_auth_transition_notification_to_player_legacy_compatible(
    player: &mut MpvAdapter,
    notification: &ControllerAuthTransitionNotification,
) {
    if controller_auth_notification_hidden_from_osd(notification) {
        return;
    }

    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let message = controller_auth_transition_notification_message_localized_legacy_compatible(
        notification,
        language.as_deref(),
    );
    emit_sorotte_player_osd_notification_legacy_compatible(
        player,
        &message,
        LegacySyncplayOsdKind::Notification,
        "controller-auth notification",
    );
}

pub(crate) fn flush_controller_auth_notifications_legacy_compatible(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
) -> anyhow::Result<()> {
    while let Some(notification) = runtime.pending_controller_auth_notification().cloned() {
        emit_controller_auth_transition_notification_to_player_legacy_compatible(
            runtime.player_mut(),
            &notification,
        );
        emit_controller_auth_transition_notification(&notification)?;
        let acknowledged = runtime.acknowledge_controller_auth_notification();
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_controller_auth_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ControllerAuthTransitionNotification) -> anyhow::Result<()>,
{
    runtime.drain_controller_auth_notifications_to_sink(|notification| notify(notification))
}
