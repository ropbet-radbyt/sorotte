use sorotte_client_core::ControllerAuthTransitionNotification;

pub fn controller_auth_transition_notification_message(
    notification: &ControllerAuthTransitionNotification,
) -> String {
    match notification {
        ControllerAuthTransitionNotification::Attempting { room } => {
            format!("Identifying as room operator in room {room}...")
        }
        ControllerAuthTransitionNotification::Succeeded { username, room, .. } => {
            format!("{username} authenticated as a room operator in room {room}")
        }
        ControllerAuthTransitionNotification::Failed { username, room, .. } => {
            format!("{username} failed to identify as a room operator in room {room}")
        }
    }
}

pub fn controller_auth_transition_notification_message_localized_legacy_compatible(
    notification: &ControllerAuthTransitionNotification,
    language: Option<&str>,
) -> String {
    match notification {
        ControllerAuthTransitionNotification::Attempting { room } => match language {
            Some("de") => format!("Identifiziere als Raumoperator in Raum {room}..."),
            Some("es") => {
                format!("Identificandose como operador de la sala en la sala {room}...")
            }
            Some("eo") => {
                format!("Identigante kiel cxambro-operatoro en cxambro {room}...")
            }
            Some("fi") => {
                format!("Tunnistaudutaan huoneen operaattoriksi huoneessa {room}...")
            }
            Some("fr") => {
                format!("Identification comme operateur de salle dans la salle {room}...")
            }
            Some("it") => {
                format!("Identificazione come operatore della stanza nella stanza {room}...")
            }
            Some("pt_PT" | "pt_BR") => {
                format!("Identificando como operador da sala na sala {room}...")
            }
            Some("tr") => {
                format!("Oda operatoru olarak {room} odasinda kimlik dogrulaniyor...")
            }
            Some("ru") => format!("Identifikatsiia kak operator komnaty v komnate {room}..."),
            Some("zh_CN") => {
                format!("Zhengzai zai fangjian {room} zhong yanzheng wei fangjian guanliyuan...")
            }
            Some("ko") => {
                format!("Bang {room}eseo bang unyeongja-ro inyong jung...")
            }
            _ => controller_auth_transition_notification_message(notification),
        },
        ControllerAuthTransitionNotification::Succeeded { username, room, .. } => match language {
            Some("de") => {
                format!("{username} wurde als Raumoperator in Raum {room} authentifiziert")
            }
            Some("es") => {
                format!("{username} se autentico como operador de la sala en la sala {room}")
            }
            Some("eo") => {
                format!("{username} sukcese identigxis kiel cxambro-operatoro en cxambro {room}")
            }
            Some("fi") => {
                format!("{username} tunnistautui huoneen operaattoriksi huoneessa {room}")
            }
            Some("fr") => {
                format!(
                    "{username} s'est authentifie comme operateur de salle dans la salle {room}"
                )
            }
            Some("it") => {
                format!(
                    "{username} si e autenticato come operatore della stanza nella stanza {room}"
                )
            }
            Some("pt_PT" | "pt_BR") => {
                format!("{username} foi autenticado como operador da sala na sala {room}")
            }
            Some("tr") => {
                format!("{username}, {room} odasinda oda operatoru olarak dogrulandi")
            }
            Some("ru") => {
                format!("{username} identifitsirovan kak operator komnaty v komnate {room}")
            }
            Some("zh_CN") => {
                format!("{username} yi zai fangjian {room} zhong yanzheng wei fangjian guanliyuan")
            }
            Some("ko") => {
                format!("{username}neun bang {room}eseo bang unyeongja-ro inyongdoeeotseumnida")
            }
            _ => controller_auth_transition_notification_message(notification),
        },
        ControllerAuthTransitionNotification::Failed { username, room, .. } => match language {
            Some("de") => format!(
                "{username} konnte sich nicht als Raumoperator in Raum {room} identifizieren"
            ),
            Some("es") => format!(
                "{username} no pudo identificarse como operador de la sala en la sala {room}"
            ),
            Some("eo") => {
                format!("{username} malsukcesis identigxi kiel cxambro-operatoro en cxambro {room}")
            }
            Some("fi") => format!(
                "{username} epaonnistui tunnistautumaan huoneen operaattoriksi huoneessa {room}"
            ),
            Some("fr") => format!(
                "{username} n'a pas pu s'identifier comme operateur de salle dans la salle {room}"
            ),
            Some("it") => format!(
                "{username} non e riuscito a identificarsi come operatore della stanza nella stanza {room}"
            ),
            Some("pt_PT" | "pt_BR") => format!(
                "{username} nao conseguiu se identificar como operador da sala na sala {room}"
            ),
            Some("tr") => {
                format!("{username}, {room} odasinda oda operatoru olarak kimligini dogrulayamadi")
            }
            Some("ru") => format!(
                "{username} ne smog identifitsirovatsia kak operator komnaty v komnate {room}"
            ),
            Some("zh_CN") => format!(
                "{username} wei neng zai fangjian {room} zhong yanzheng wei fangjian guanliyuan"
            ),
            Some("ko") => format!(
                "{username}neun bang {room}eseo bang unyeongja-ro inyonghaji moshaetseumnida"
            ),
            _ => controller_auth_transition_notification_message(notification),
        },
    }
}

pub fn controller_auth_notification_hidden_from_osd(
    notification: &ControllerAuthTransitionNotification,
) -> bool {
    match notification {
        ControllerAuthTransitionNotification::Attempting { .. } => false,
        ControllerAuthTransitionNotification::Succeeded { hide_from_osd, .. }
        | ControllerAuthTransitionNotification::Failed { hide_from_osd, .. } => *hide_from_osd,
    }
}
