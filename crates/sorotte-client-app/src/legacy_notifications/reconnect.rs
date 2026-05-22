use sorotte_client_core::ReconnectTransitionNotification;

pub fn reconnect_transition_notification_message(
    notification: &ReconnectTransitionNotification,
) -> String {
    match notification {
        ReconnectTransitionNotification::Attempting {
            retries,
            delay_seconds,
        } => format!(
            "Connection with server lost, attempting to reconnect (retry={retries}, delay_seconds={delay_seconds:.3})"
        ),
        ReconnectTransitionNotification::Connected => "Reconnected to server".to_owned(),
        ReconnectTransitionNotification::Disconnected => {
            "Connection with server lost, reconnect attempts exhausted".to_owned()
        }
        ReconnectTransitionNotification::RestoringState => {
            "Restoring local state after reconnect...".to_owned()
        }
        ReconnectTransitionNotification::StateRestoreValidationMismatch {
            local_paused,
            room_paused,
            local_position,
            room_position,
            position_diff_seconds,
        } => format!(
            "Reconnect state restore validation mismatch; correcting local player: player(paused={local_paused}, position={local_position:.3}) room(paused={room_paused}, position={room_position:.3}) diff={position_diff_seconds:.3}"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRetryScheduled {
            attempt,
            max_attempts,
            cooldown_ticks,
        } => format!(
            "Reconnect state restore correction failed; scheduling retry (attempt={attempt}/{max_attempts}, cooldown_ticks={cooldown_ticks})"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRetriesExhausted {
            attempts,
            max_attempts,
        } => format!(
            "Reconnect state restore correction failed; retry budget exhausted (attempts={attempts}, max_attempts={max_attempts}), stopping auto-correction for this restore cycle"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionDisabledAfterRepeatedMismatches {
            consecutive_mismatch_cycles,
            disable_after_mismatch_cycles,
        } => format!(
            "Reconnect state restore correction disabled after repeated mismatches (consecutive_mismatch_cycles={consecutive_mismatch_cycles}, threshold={disable_after_mismatch_cycles})"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownSuppressed {
            remaining_reconnect_cycles_after_this_cycle,
        } => format!(
            "Reconnect state restore correction suppressed for recovery cooldown (remaining_reconnect_cycles_after_this_cycle={remaining_reconnect_cycles_after_this_cycle})"
        ),
        ReconnectTransitionNotification::StateRestoreValidationCorrectionRecoveryCooldownReenabled => {
            "Reconnect state restore correction re-enabled after recovery cooldown".to_owned()
        }
        ReconnectTransitionNotification::RestoringPlaylist => {
            "Restoring playlist on reconnect...".to_owned()
        }
    }
}

pub fn reconnect_transition_notification_message_localized_legacy_compatible(
    notification: &ReconnectTransitionNotification,
    language: Option<&str>,
) -> String {
    match notification {
        ReconnectTransitionNotification::Attempting {
            retries,
            delay_seconds,
        } => match language {
            Some("de") => format!(
                "Verbindung zum Server verloren, erneuter Verbindungsversuch (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("es") => format!(
                "Conexion con el servidor perdida, intentando reconectar (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("eo") => format!(
                "Konekto al servilo perdita, provas rekonekti (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("fi") => format!(
                "Yhteys palvelimeen katkesi, yritetaan yhdistaa uudelleen (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("fr") => format!(
                "Connexion au serveur perdue, tentative de reconnexion (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("it") => format!(
                "Connessione al server persa, tentativo di riconnessione (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("pt_PT" | "pt_BR") => format!(
                "Conexao com o servidor perdida, tentando reconectar (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("tr") => format!(
                "Sunucu baglantisi kesildi, yeniden baglanmaya calisiliyor (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("ru") => format!(
                "Soedinenie s serverom poterianno, popytka povtornogo podkliucheniia (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("zh_CN") => format!(
                "Yu fuwuqi de lianjie yi diu shi, zhengzai changshi chongxin lianjie (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            Some("ko") => format!(
                "Seobeowa-ui yeongyeori kkeun-eojyeosseum, dasi yeongyeol si-do jung (retry={retries}, delay_seconds={delay_seconds:.3})"
            ),
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::Connected => match language {
            Some("de") => "Erneut mit dem Server verbunden".to_owned(),
            Some("es") => "Reconectado al servidor".to_owned(),
            Some("eo") => "Rekonektita al servilo".to_owned(),
            Some("fi") => "Yhdistetty uudelleen palvelimeen".to_owned(),
            Some("fr") => "Reconnecte au serveur".to_owned(),
            Some("it") => "Riconnesso al server".to_owned(),
            Some("pt_PT" | "pt_BR") => "Reconectado ao servidor".to_owned(),
            Some("tr") => "Sunucuya yeniden baglanildi".to_owned(),
            Some("ru") => "Povtornoe podkliuchenie k serveru vypolneno".to_owned(),
            Some("zh_CN") => "Yi chongxin lianjie dao fuwuqi".to_owned(),
            Some("ko") => "Seobeoe dasi yeongyeoldoeeotseumnida".to_owned(),
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::Disconnected => match language {
            Some("de") => {
                "Verbindung zum Server verloren, Wiederverbindungsversuche erschoepft"
                    .to_owned()
            }
            Some("es") => {
                "Conexion con el servidor perdida, intentos de reconexion agotados".to_owned()
            }
            Some("eo") => {
                "Konekto al servilo perdita, rekonektaj provoj eluzitaj".to_owned()
            }
            Some("fi") => {
                "Yhteys palvelimeen katkesi, uudelleenyhdistamisyritykset loppuivat"
                    .to_owned()
            }
            Some("fr") => {
                "Connexion au serveur perdue, tentatives de reconnexion epuisees".to_owned()
            }
            Some("it") => {
                "Connessione al server persa, tentativi di riconnessione esauriti".to_owned()
            }
            Some("pt_PT" | "pt_BR") => {
                "Conexao com o servidor perdida, tentativas de reconexao esgotadas".to_owned()
            }
            Some("tr") => {
                "Sunucu baglantisi kesildi, yeniden baglanma denemeleri tukendi"
                    .to_owned()
            }
            Some("ru") => {
                "Soedinenie s serverom poterianno, popytki povtornogo podkliucheniia ischerpany"
                    .to_owned()
            }
            Some("zh_CN") => {
                "Yu fuwuqi de lianjie yi diu shi, chongxin lianjie changshi yongjin".to_owned()
            }
            Some("ko") => {
                "Seobeowa-ui yeongyeori kkeun-eojyeosseum, dasi yeongyeol si-do-reul modu sayonghaetseumnida"
                    .to_owned()
            }
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::RestoringState => match language {
            Some("de") => "Lokalen Status nach Wiederverbindung wiederherstellen...".to_owned(),
            Some("es") => "Restaurando estado local tras la reconexion...".to_owned(),
            Some("eo") => "Restarigante lokan staton post rekonekto...".to_owned(),
            Some("fi") => "Palautetaan paikallinen tila uudelleenyhdistamisen jalkeen..."
                .to_owned(),
            Some("fr") => "Restauration de l'etat local apres reconnexion...".to_owned(),
            Some("it") => "Ripristino dello stato locale dopo la riconnessione...".to_owned(),
            Some("pt_PT" | "pt_BR") => {
                "Restaurando estado local apos a reconexao...".to_owned()
            }
            Some("tr") => "Yeniden baglanti sonrasi yerel durum geri yukleniyor...".to_owned(),
            Some("ru") => {
                "Vosstanovlenie lokalnogo sostoianiia posle povtornogo podkliucheniia..."
                    .to_owned()
            }
            Some("zh_CN") => "Zhengzai zai chongxin lianjie hou huifu bendi zhuangtai..."
                .to_owned(),
            Some("ko") => {
                "Dasi yeongyeol hu lokal sangtaereul bokguhaneun jung...".to_owned()
            }
            _ => reconnect_transition_notification_message(notification),
        },
        ReconnectTransitionNotification::RestoringPlaylist => match language {
            Some("de") => "Playlist nach Wiederverbindung wiederherstellen...".to_owned(),
            Some("es") => "Restaurando lista de reproduccion tras la reconexion...".to_owned(),
            Some("eo") => "Restarigante ludliston post rekonekto...".to_owned(),
            Some("fi") => {
                "Palautetaan soittolista uudelleenyhdistamisen jalkeen...".to_owned()
            }
            Some("fr") => "Restauration de la liste de lecture apres reconnexion...".to_owned(),
            Some("it") => "Ripristino della playlist dopo la riconnessione...".to_owned(),
            Some("pt_PT" | "pt_BR") => {
                "Restaurando lista de reproducao apos a reconexao...".to_owned()
            }
            Some("tr") => {
                "Yeniden baglanti sonrasi oynatma listesi geri yukleniyor...".to_owned()
            }
            Some("ru") => {
                "Vosstanovlenie spiska vosproizvedeniia posle povtornogo podkliucheniia..."
                    .to_owned()
            }
            Some("zh_CN") => "Zhengzai zai chongxin lianjie hou huifu bofang liebiao..."
                .to_owned(),
            Some("ko") => {
                "Dasi yeongyeol hu jaesaeng mongnog-eul bokguhaneun jung...".to_owned()
            }
            _ => reconnect_transition_notification_message(notification),
        },
        _ => reconnect_transition_notification_message(notification),
    }
}
