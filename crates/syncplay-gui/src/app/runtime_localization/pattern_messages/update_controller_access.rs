use super::super::*;

pub(super) fn localize_update_controller_access_message(
    message: &str,
    language: Option<&str>,
) -> Option<String> {
    if let Some(timestamp) = message
        .strip_prefix("Checked at: ")
        .and_then(|value| value.strip_suffix(" UTC"))
    {
        return Some(localized_update_checked_at_line_legacy_compatible(
            language, timestamp,
        ));
    }
    if let Some((username, room)) = message
        .strip_suffix('.')
        .and_then(|value| value.split_once(" received controller access for "))
    {
        return Some(with_terminal_period(&format!(
            "{username} {} {room}",
            localized_literal(
                language,
                "received controller access for",
                "erhielt Controller-Zugriff fuer",
                "recibio acceso de controlador para",
                "ricevis regantan aliron por",
                "sai ohjaajaoikeudet kohteelle",
                "a recu l'acces controleur pour",
                "ha ricevuto l'accesso controller per",
                "recebeu acesso de controlador para",
                "icin denetleyici erisimi aldi",
                "poluchil dostup kontrolera dlia",
                "huode le kongzhizhe quanxian gei",
                "daehae kontroller jeobgeun gwoneul badasseumnida"
            )
        )));
    }
    if let Some((username, room)) = message
        .strip_prefix("Controller access failed for ")
        .and_then(|value| value.strip_suffix('.'))
        .and_then(|value| value.split_once(" in "))
    {
        return Some(with_terminal_period(&format!(
            "{} {username} {} {room}",
            localized_literal(
                language,
                "Controller access failed for",
                "Controller-Zugriff fehlgeschlagen fuer",
                "Fallo el acceso de controlador para",
                "Reganta aliro malsukcesis por",
                "Ohjaajaoikeuksien pyynto epaonnistui kohteelle",
                "L'acces controleur a echoue pour",
                "Accesso controller non riuscito per",
                "Falha no acesso de controlador para",
                "Denetleyici erisimi basarisiz oldu:",
                "Dostup kontrolera ne udalsia dlia",
                "Kongzhizhe quanxian qingqiu shibai yu",
                "kontroller jeobgeuni silpaehaetseumnida:"
            ),
            localized_literal(
                language,
                "in",
                "in",
                "en",
                "en",
                "kohteessa",
                "dans",
                "in",
                "em",
                "odasinda",
                "v",
                "zai",
                "bangeseo"
            )
        )));
    }
    None
}
