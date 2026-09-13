use sorotte_client_app::app_boundary::language::normalized_runtime_language_tag;

use super::shell_state::SorotteGuiShellAppState;

mod exact_messages;
mod generic_errors;
mod pattern_messages;
#[cfg(test)]
mod tests;

#[allow(clippy::too_many_arguments)]
fn localized_literal(
    language: Option<&str>,
    en: &'static str,
    de: &'static str,
    es: &'static str,
    eo: &'static str,
    fi: &'static str,
    fr: &'static str,
    it: &'static str,
    pt: &'static str,
    tr: &'static str,
    ru: &'static str,
    zh_cn: &'static str,
    ko: &'static str,
) -> &'static str {
    match language {
        Some("de") => de,
        Some("es") => es,
        Some("eo") => eo,
        Some("fi") => fi,
        Some("fr") => fr,
        Some("it") => it,
        Some("pt_PT" | "pt_BR") => pt,
        Some("tr") => tr,
        Some("ru") => ru,
        Some("zh_CN") => zh_cn,
        Some("ko") => ko,
        _ => en,
    }
}

pub(super) fn normalized_runtime_language_tag_or_default(language: Option<&str>) -> &'static str {
    language
        .and_then(normalized_runtime_language_tag)
        .unwrap_or("en")
}

impl SorotteGuiShellAppState {
    pub(super) fn runtime_language_tag(&self) -> &'static str {
        normalized_runtime_language_tag_or_default(self.active_application_language.as_deref())
    }
}

pub(super) fn localized_sorotte_uptodate_message(language: Option<&str>) -> &'static str {
    localized_literal(
        language,
        "Sorotte is up to date",
        "Sorotte ist auf dem neuesten Stand",
        "Sorotte esta actualizado",
        "Sorotte estas gxisdata",
        "Sorotte on ajan tasalla",
        "Sorotte est a jour",
        "Sorotte e aggiornato",
        "O Sorotte esta atualizado",
        "Sorotte guncel",
        "Sorotte obnovlen do poslednei versii",
        "Sorotte yi shi zuixin banben",
        "Sorotteneun choesin sangtaeimnida",
    )
}

pub(super) fn localized_sorotte_update_available_message(language: Option<&str>) -> &'static str {
    localized_literal(
        language,
        "A new version of Sorotte is available. Do you want to visit the release page?",
        "Eine neue Version von Sorotte ist verfuegbar. Moechten Sie die Release-Seite besuchen?",
        "Hay una nueva version de Sorotte disponible. Desea visitar la pagina de lanzamiento?",
        "Nova versio de Sorotte disponeblas. Chu vi volas viziti la eldonan paghon?",
        "Uusi Sorotte-versio on saatavilla. Haluatko avata julkaisusivun?",
        "Une nouvelle version de Sorotte est disponible. Voulez-vous visiter la page de publication?",
        "E disponibile una nuova versione di Sorotte. Vuoi visitare la pagina di rilascio?",
        "Uma nova versao do Sorotte esta disponivel. Deseja visitar a pagina de lancamento?",
        "Sorotte'nin yeni bir surumu mevcut. Surum sayfasini ziyaret etmek ister misiniz?",
        "Dostupna novaia versiia Sorotte. Otkryt stranicu vypuska?",
        "You xin de Sorotte banben ke yong. Yao fangwen fabu yemian ma?",
        "Sorotte-ui saeroun beojeoni isseumnida. baepo peijireul bangmunhasigesseumnikka?",
    )
}

#[cfg(test)]
pub(super) fn localized_public_server_list_failed_message(language: Option<&str>) -> &'static str {
    localized_literal(
        language,
        "Failed to load public server list. Please visit https://www.syncplay.pl/ in your browser.",
        "Die Liste der oeffentlichen Server konnte nicht geladen werden. Bitte besuchen Sie https://www.syncplay.pl/ in Ihrem Browser.",
        "No se pudo cargar la lista de servidores publicos. Visite https://www.syncplay.pl/ en su navegador.",
        "Malsukcesis sxargi la liston de publikaj serviloj. Bonvolu viziti https://www.syncplay.pl/ en via retumilo.",
        "Julkisten palvelinten listaa ei voitu ladata. Kay osoitteessa https://www.syncplay.pl/ selaimessasi.",
        "Echec du chargement de la liste des serveurs publics. Veuillez visiter https://www.syncplay.pl/ dans votre navigateur.",
        "Impossibile caricare l'elenco dei server pubblici. Visita https://www.syncplay.pl/ nel browser.",
        "Falha ao carregar a lista de servidores publicos. Visite https://www.syncplay.pl/ no navegador.",
        "Genel sunucu listesi yuklenemedi. Lutfen tarayicinizda https://www.syncplay.pl/ adresini ziyaret edin.",
        "Ne udalos zagruzit spisok publichnykh serverov. Pozhaluista, otkroite https://www.syncplay.pl/ v brauzere.",
        "Wu fa jiazai gonggong fuwuqi liebiao. Qing zai liulanqi zhong fangwen https://www.syncplay.pl/ .",
        "gonggae seobeo mongnog-eul bulleo-oji moshaetseumnida. beuraujeoeseo https://www.syncplay.pl/ reul yeoreojuseyo.",
    )
}

pub(super) fn localized_update_checked_at_line(language: Option<&str>, timestamp: &str) -> String {
    format!(
        "{} {timestamp} UTC",
        localized_literal(
            language,
            "Checked at:",
            "Geprueft um:",
            "Comprobado a las:",
            "Kontrolite je:",
            "Tarkistettu:",
            "Verifie a :",
            "Controllato alle:",
            "Verificado em:",
            "Kontrol edildi:",
            "Provereno v:",
            "Jiancha shijian:",
            "hwagin sigan:",
        )
    )
}

fn with_terminal_period(message: &str) -> String {
    if message.ends_with('.') {
        message.to_owned()
    } else {
        format!("{message}.")
    }
}

fn with_original_terminal_period(original: &str, localized: &str) -> String {
    if original.ends_with('.') {
        with_terminal_period(localized)
    } else {
        localized.to_owned()
    }
}

fn strip_period(message: &str) -> &str {
    message.strip_suffix('.').unwrap_or(message)
}

fn localize_ready_state(language: Option<&str>, ready: bool) -> &'static str {
    if ready {
        localized_literal(
            language,
            "ready",
            "bereit",
            "listo",
            "preta",
            "valmis",
            "pret",
            "pronto",
            "pronto",
            "hazir",
            "gotov",
            "yi zhunbei",
            "junbi",
        )
    } else {
        localized_literal(
            language,
            "not ready",
            "nicht bereit",
            "no listo",
            "ne preta",
            "ei valmis",
            "pas pret",
            "non pronto",
            "nao pronto",
            "hazir degil",
            "ne gotov",
            "wei zhunbei",
            "junbi an doem",
        )
    }
}

fn localize_role_state(language: Option<&str>, controller: bool) -> &'static str {
    if controller {
        localized_literal(
            language,
            "controller",
            "Controller",
            "controlador",
            "reganto",
            "ohjaaja",
            "controleur",
            "controller",
            "controlador",
            "denetleyici",
            "kontroler",
            "kongzhizhe",
            "kontroller",
        )
    } else {
        localized_literal(
            language,
            "participant",
            "Teilnehmer",
            "participante",
            "partoprenanto",
            "osallistuja",
            "participant",
            "partecipante",
            "participante",
            "katilimci",
            "uchastnik",
            "canyuzhe",
            "chamyeoja",
        )
    }
}

pub(super) fn localize_gui_runtime_message(message: &str, language: Option<&str>) -> String {
    let language = Some(normalized_runtime_language_tag_or_default(language));
    exact_messages::localize_exact_message(message, language)
        .or_else(|| pattern_messages::localize_pattern_message(message, language))
        .or_else(|| generic_errors::localize_generic_error_message(message, language))
        .unwrap_or_else(|| message.to_owned())
}
