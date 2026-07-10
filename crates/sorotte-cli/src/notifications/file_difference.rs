use super::*;

#[cfg(test)]
pub(crate) fn format_file_difference_summary(summary: FileDifferenceSummary) -> Option<String> {
    shared_format_file_difference_summary(summary)
}

#[cfg(test)]
pub(crate) fn localized_file_difference_summary_legacy_compatible(
    summary: &str,
    language: Option<&str>,
) -> String {
    shared_localized_file_difference_summary_legacy_compatible(summary, language)
}

pub(crate) fn emit_file_difference_notification(summary: &str) -> anyhow::Result<()> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    println!(
        "{}",
        shared_localized_file_difference_notification_line_legacy_compatible(
            summary,
            language.as_deref(),
        )
    );
    Ok(())
}

#[cfg(test)]
pub(crate) fn flush_file_difference_notifications_to_sink<F>(
    runtime: &ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    state: &mut FileDifferenceNotificationState,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> anyhow::Result<()>,
{
    if let Some(summary) = shared_next_file_difference_notification_summary_legacy_compatible(
        state,
        runtime.session().file_differences_for_current_room(),
    ) {
        notify(summary.as_str())?;
    }

    Ok(())
}

fn file_difference_notification_message_localized_legacy_compatible(
    summary: &str,
    language: Option<&str>,
) -> String {
    shared_localized_file_difference_notification_line_legacy_compatible(summary, language)
}

fn emit_file_difference_notification_to_player_legacy_compatible(
    player: &mut MpvAdapter,
    summary: &str,
) {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let message = file_difference_notification_message_localized_legacy_compatible(
        summary,
        language.as_deref(),
    );
    emit_sorotte_player_osd_notification_legacy_compatible(
        player,
        &message,
        LegacySyncplayOsdKind::Notification,
        "file difference notification",
    );
}

pub(crate) fn flush_file_difference_notifications_legacy_compatible<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    state: &mut FileDifferenceNotificationState,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&str) -> anyhow::Result<()>,
{
    if let Some(summary) = shared_next_file_difference_notification_summary_legacy_compatible(
        state,
        runtime.session().file_differences_for_current_room(),
    ) {
        runtime.with_player_io(|player| {
            emit_file_difference_notification_to_player_legacy_compatible(player, &summary);
        });
        notify(summary.as_str())?;
    }

    Ok(())
}
