use super::*;

pub(crate) fn chat_notification_message(notification: &ChatNotification) -> String {
    match notification {
        ChatNotification::Message { username, message } => match username.as_deref() {
            Some(username) => format!("<{username}> {message}"),
            None => message.clone(),
        },
    }
}

fn emit_chat_notification(notification: &ChatNotification) -> anyhow::Result<()> {
    println!("{}", chat_notification_message(notification));
    Ok(())
}

fn emit_chat_notification_to_player_legacy_compatible(
    player: &mut MpvAdapter,
    notification: &ChatNotification,
) {
    emit_sorotte_player_chat_notification_legacy_compatible(
        player,
        &chat_notification_message(notification),
    );
}

pub(crate) fn flush_chat_notifications_legacy_compatible(
    runtime: &mut ClientApplication<MpvAdapter>,
) -> anyhow::Result<()> {
    while let Some(notification) = runtime.pending_chat_notification().cloned() {
        runtime.with_player_io(|player| {
            emit_chat_notification_to_player_legacy_compatible(player, &notification);
        });
        emit_chat_notification(&notification)?;
        let acknowledged = runtime.acknowledge_chat_notification();
        debug_assert!(acknowledged.is_some());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_chat_notifications_to_sink<F>(
    runtime: &mut ClientApplication<MpvAdapter>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ChatNotification) -> anyhow::Result<()>,
{
    runtime.drain_chat_notifications_to_sink(|notification| notify(notification))
}
