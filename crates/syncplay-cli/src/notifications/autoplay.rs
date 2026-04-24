use super::*;

pub(crate) fn autoplay_countdown_notification_message_localized_legacy_compatible(
    notification: &AutoplayCountdownNotification,
    language: Option<&str>,
) -> String {
    match language {
        Some("de") => format!(
            "Autoplay-Countdown: bereit_benutzer={} sekunden_uebrig={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("es") => format!(
            "Cuenta regresiva de autoplay: usuarios_listos={} segundos_restantes={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("eo") => format!(
            "Auta luda retronombrado: pretaj_uzantoj={} ceteraj_sekundoj={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("fi") => format!(
            "Autoplayn laskenta: valmiit_kayttajat={} sekuntia_jaljella={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("fr") => format!(
            "Compte a rebours autoplay : utilisateurs_prets={} secondes_restantes={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("it") => format!(
            "Conto alla rovescia autoplay: utenti_pronti={} secondi_rimanenti={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("pt_PT" | "pt_BR") => format!(
            "Contagem regressiva do autoplay: usuarios_prontos={} segundos_restantes={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("tr") => format!(
            "Autoplay geri sayim: hazir_kullanicilar={} kalan_saniye={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("ru") => format!(
            "Obratnyi otschet autoplay: gotovye_polzovateli={} ostalos_sekund={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("zh_CN") => format!(
            "Autoplay daojishi: zhunbei_yonghu={} shengyu_miaoshu={}",
            notification.ready_user_count, notification.seconds_left
        ),
        Some("ko") => format!(
            "Autoplay kaunteudaun: junbi_sayongja={} namaeun_cho={}",
            notification.ready_user_count, notification.seconds_left
        ),
        _ => format!(
            "autoplay countdown: ready_users={} seconds_left={}",
            notification.ready_user_count, notification.seconds_left
        ),
    }
}

pub(crate) fn emit_autoplay_countdown_notification(
    notification: &AutoplayCountdownNotification,
) -> anyhow::Result<()> {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    println!(
        "{}",
        autoplay_countdown_notification_message_localized_legacy_compatible(
            notification,
            language.as_deref(),
        )
    );
    Ok(())
}

fn emit_autoplay_countdown_notification_to_player_legacy_compatible(
    player: &mut MpvAdapter,
    notification: &AutoplayCountdownNotification,
) {
    let language = current_legacy_runtime_language_tag_legacy_compatible();
    let message = autoplay_countdown_notification_message_localized_legacy_compatible(
        notification,
        language.as_deref(),
    );
    emit_syncplay_player_osd_notification_legacy_compatible(
        player,
        &message,
        LegacySyncplayOsdKind::Alert,
        "autoplay notification",
    );
}

#[cfg(test)]
pub(crate) fn flush_autoplay_notifications_to_sink<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
{
    runtime.drain_autoplay_notifications_to_sink(|notification| notify(notification))
}

pub(crate) fn flush_autoplay_notifications_legacy_compatible<F>(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    notify: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&AutoplayCountdownNotification) -> anyhow::Result<()>,
{
    for notification in runtime.drain_autoplay_notifications() {
        emit_autoplay_countdown_notification_to_player_legacy_compatible(
            runtime.player_mut(),
            &notification,
        );
        notify(&notification)?;
    }
    Ok(())
}
