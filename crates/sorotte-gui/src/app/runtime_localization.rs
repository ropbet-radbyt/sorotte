use sorotte_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;

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
        .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        .unwrap_or("en")
}

impl SorotteGuiShellAppState {
    pub(super) fn runtime_language_tag_legacy_compatible(&self) -> &'static str {
        normalized_runtime_language_tag_or_default(
            self.configuration.to_stored_settings().language.as_deref(),
        )
    }
}

pub(super) fn localized_update_notice_available_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    localized_literal(
        language,
        "An update notice is available for this client build.",
        "Ein Update-Hinweis ist fuer diesen Client-Build verfuegbar.",
        "Hay un aviso de actualizacion disponible para esta compilacion del cliente.",
        "Ghisdiga avizo haveblas por ci tiu klienta konstruo.",
        "Talle asiakasversiolle on saatavilla paivityshuomautus.",
        "Un avis de mise a jour est disponible pour cette version du client.",
        "E disponibile un avviso di aggiornamento per questa build del client.",
        "Ha um aviso de atualizacao disponivel para esta compilacao do cliente.",
        "Bu istemci derlemesi icin bir guncelleme bildirimi mevcut.",
        "Dlia etoi sborki klienta dostupno uvedomlenie ob obnovlenii.",
        "Ci kehu duan goujian you ke yong de gengxin tongzhi.",
        "I keullaieonteu deobeureul wihan eobdeiteu annae-ga isseumnida.",
    )
}

pub(super) fn localized_sorotte_uptodate_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
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

pub(super) fn localized_sorotte_update_available_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
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
pub(super) fn localized_update_check_failed_message_legacy_compatible(
    language: Option<&str>,
    version: &str,
) -> String {
    localized_literal(
        language,
        "Could not automatically check whether Sorotte {} is up to date. Want to visit https://syncplay.pl/ to manually check for updates?",
        "Es konnte nicht automatisch geprueft werden, ob Sorotte {} aktuell ist. Moechten Sie https://syncplay.pl/ besuchen, um manuell nach Updates zu suchen?",
        "No se pudo comprobar automaticamente si Sorotte {} esta actualizado. Desea visitar https://syncplay.pl/ para comprobar manualmente si hay actualizaciones?",
        "Ne eblis auxtomate kontroli chu Sorotte {} estas gxisdata. Chu vi volas viziti https://syncplay.pl/ por mane kontroli gxisdatigojn?",
        "Ei voitu tarkistaa automaattisesti, onko Sorotte {} ajan tasalla. Haluatko kayda osoitteessa https://syncplay.pl/ tarkistaaksesi paivitykset manuaalisesti?",
        "Impossible de verifier automatiquement si Sorotte {} est a jour. Voulez-vous visiter https://syncplay.pl/ pour verifier manuellement les mises a jour?",
        "Impossibile verificare automaticamente se Sorotte {} e aggiornato. Vuoi visitare https://syncplay.pl/ per controllare manualmente gli aggiornamenti?",
        "Nao foi possivel verificar automaticamente se o Sorotte {} esta atualizado. Deseja visitar https://syncplay.pl/ para verificar atualizacoes manualmente?",
        "Sorotte {}'nin guncel olup olmadigi otomatik olarak denetlenemedi. Guncellemeleri elle kontrol etmek icin https://syncplay.pl/ adresini ziyaret etmek ister misiniz?",
        "Ne udalos avtomaticheski proverit, obnovlen li Sorotte {}. Hotite pereiti na https://syncplay.pl/ dlia ruchnoi proverki obnovlenii?",
        "Wu fa zidong jiancha Sorotte {} shifou wei zuixin banben. Yao fangwen https://syncplay.pl/ shoudong jiancha gengxin ma?",
        "Sorotte {}ga choesin beojeoninji jadongeuro hwaginhal su eopseotseumnida. susdong-euro eobdeiteureul hwaginhagi wihae https://syncplay.pl/ reul bangmunhasigesseumnikka?",
    )
    .replace("{}", version)
}

#[cfg(test)]
pub(super) fn localized_public_server_list_failed_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
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

pub(super) fn localized_update_checked_at_line_legacy_compatible(
    language: Option<&str>,
    timestamp: &str,
) -> String {
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

pub(super) fn localized_update_dismiss_hint_line_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    localized_literal(
        language,
        "Dismiss it here or trigger another update check from the same modal.",
        "Schliessen Sie den Hinweis hier oder starten Sie aus demselben Dialog eine neue Update-Pruefung.",
        "Descartelo aqui o inicie otra comprobacion de actualizaciones desde el mismo cuadro.",
        "Fermu gxin cxi tie au lanccu alian gxisdatigan kontrolon el la sama dialogo.",
        "Sulje ilmoitus taalta tai kaynnista uusi paivitystarkistus samasta ikkunasta.",
        "Fermez cet avis ici ou lancez une nouvelle verification depuis la meme fenetre.",
        "Chiudi questo avviso qui oppure avvia un nuovo controllo aggiornamenti dalla stessa finestra.",
        "Feche este aviso aqui ou inicie outra verificacao de atualizacoes na mesma janela.",
        "Bu bildirimi burada kapatin ya da ayni pencereden yeni bir guncelleme denetimi baslatin.",
        "Zakroite eto uvedomlenie zdes ili zapustite novuio proverku obnovlenii iz etogo zhe okna.",
        "Zai ci chuguan ci tongzhi huo cong tongyi chuangkou chufa ling yi ci gengxin jiancha.",
        "yeogieseo i annae-reul datgeona gateun chang-eseo dasi eobdeiteu geomsareul silhaenghasipsio.",
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

pub(super) fn localize_gui_runtime_message_legacy_compatible(
    message: &str,
    language: Option<&str>,
) -> String {
    let language = Some(normalized_runtime_language_tag_or_default(language));
    exact_messages::localize_exact_message(message, language)
        .or_else(|| pattern_messages::localize_pattern_message(message, language))
        .or_else(|| generic_errors::localize_generic_error_message(message, language))
        .unwrap_or_else(|| message.to_owned())
}
