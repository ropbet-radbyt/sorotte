use super::super::*;

pub(super) fn localize_user_status_message(
    message: &str,
    language: Option<&str>,
) -> Option<String> {
    if let Some(rest) = message.strip_prefix("User readiness updated: ")
        && let Some((username, state)) = rest
            .strip_suffix('.')
            .and_then(|value| value.split_once(" -> "))
    {
        return Some(with_terminal_period(&format!(
            "{}: {username} -> {}",
            localized_literal(
                language,
                "User readiness updated",
                "Benutzerbereitschaft aktualisiert",
                "Disponibilidad de usuario actualizada",
                "Preteco de uzanto gxisdatigita",
                "Kayttajan valmiustila paivitetty",
                "Etat de preparation de l'utilisateur mis a jour",
                "Stato di prontezza utente aggiornato",
                "Prontidao do usuario atualizada",
                "Kullanici hazirlik durumu guncellendi",
                "Gotovnost polzovatelia obnovlena",
                "Yonghu zhunbei zhuangtai yi gengxin",
                "sayongja junbi sangtaega eobdeiteu-doeeotseumnida"
            ),
            localize_ready_state(language, state == "ready"),
        )));
    }
    if let Some(rest) = message.strip_prefix("Controller status updated: ")
        && let Some((username, role)) = rest
            .strip_suffix('.')
            .and_then(|value| value.split_once(" -> "))
    {
        return Some(with_terminal_period(&format!(
            "{}: {username} -> {}",
            localized_literal(
                language,
                "Controller status updated",
                "Controller-Status aktualisiert",
                "Estado de controlador actualizado",
                "Stato de reganto gxisdatigita",
                "Ohjaajatila paivitetty",
                "Statut de controleur mis a jour",
                "Stato controller aggiornato",
                "Status do controlador atualizado",
                "Denetleyici durumu guncellendi",
                "Status kontrolera obnovlen",
                "Kongzhizhe zhuangtai yi gengxin",
                "kontroller sangtaega eobdeiteu-doeeotseumnida"
            ),
            localize_role_state(language, role == "controller"),
        )));
    }
    None
}
