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
    emit_syncplay_player_chat_notification_legacy_compatible(
        player,
        &chat_notification_message(notification),
    );
}

pub(crate) fn flush_chat_notifications_legacy_compatible(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
) -> anyhow::Result<()> {
    for notification in runtime.drain_chat_notifications() {
        emit_chat_notification_to_player_legacy_compatible(runtime.player_mut(), &notification);
        emit_chat_notification(&notification)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_chat_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&ChatNotification) -> anyhow::Result<()>,
{
    runtime.drain_chat_notifications_to_sink(|notification| notify(notification))
}
