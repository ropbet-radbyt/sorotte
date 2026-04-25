use super::*;

pub(super) fn localize_generic_error_message(
    message: &str,
    language: Option<&str>,
) -> Option<String> {
    if language == Some("en") {
        return None;
    }
    if message == "Another GUI operation is already in progress." {
        return Some(with_terminal_period(localized_literal(
            language,
            "Another GUI operation is already in progress",
            "Ein anderer GUI-Vorgang laeuft bereits",
            "Otra operacion de la GUI ya esta en curso",
            "Alia GUI-operacio jam estas en progreso",
            "Toinen GUI-toiminto on jo kaynnissa",
            "Une autre operation GUI est deja en cours",
            "Un'altra operazione GUI e gia in corso",
            "Outra operacao da GUI ja esta em andamento",
            "Baska bir GUI islemi zaten suruyor",
            "Uje vypolniaetsia drugaia operatsiia GUI",
            "Ling yi xiang GUI caozuo yi zai jinxing zhong",
            "dareun GUI jageobi imi jinhaeng jungimnida",
        )));
    }
    if let Some(thing) = message
        .strip_prefix("No ")
        .and_then(|value| value.strip_suffix(" is currently in progress."))
    {
        return Some(with_terminal_period(&format!(
            "{} \"{thing}\"",
            localized_literal(
                language,
                "No active operation for",
                "Kein aktiver Vorgang fuer",
                "No hay una operacion activa para",
                "Ne ekzistas aktiva operacio por",
                "Ei aktiivista toimintoa kohteelle",
                "Aucune operation active pour",
                "Nessuna operazione attiva per",
                "Nenhuma operacao ativa para",
                "Icin etkin islem yok:",
                "Net aktivnoi operatsii dlia",
                "Meiyou huodong caozuo yongyu",
                "daehae jinhaeng jungin jageobi eopseumnida:"
            )
        )));
    }
    if let Some(thing) = message
        .strip_prefix("No ")
        .and_then(|value| value.strip_suffix(" is currently selected."))
    {
        return Some(with_terminal_period(&format!(
            "{} \"{thing}\"",
            localized_literal(
                language,
                "Nothing is currently selected for",
                "Nichts ist derzeit ausgewaehlt fuer",
                "Nada esta seleccionado actualmente para",
                "Nenio estas nun elektita por",
                "Mitaan ei ole valittuna kohteelle",
                "Rien n'est actuellement selectionne pour",
                "Niente e attualmente selezionato per",
                "Nada esta selecionado atualmente para",
                "Su anda secili degil:",
                "Seichas nichego ne vybrano dlia",
                "Dangqian meiyou wei ci xuanze renhe xiangmu",
                "hyeonjae daeehaye seontaekdoen hangmogi eopseumnida:"
            )
        )));
    }
    if let Some(thing) = message.strip_suffix(" cannot be empty.") {
        return Some(with_terminal_period(&format!(
            "\"{thing}\" {}",
            localized_literal(
                language,
                "cannot be empty",
                "darf nicht leer sein",
                "no puede estar vacio",
                "ne povas esti malplena",
                "ei voi olla tyhja",
                "ne peut pas etre vide",
                "non puo essere vuoto",
                "nao pode estar vazio",
                "bos olamaz",
                "ne mozhet byt pustym",
                "buneng wei kong",
                "bieo isseul su eopseumnida"
            )
        )));
    }
    None
}
