use syncplay_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;

use super::shell_state::SyncplayGuiShellAppState;

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

impl SyncplayGuiShellAppState {
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

pub(super) fn localized_syncplay_uptodate_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    localized_literal(
        language,
        "Syncplay is up to date",
        "Syncplay ist auf dem neuesten Stand",
        "Syncplay esta actualizado",
        "Syncplay estas gxisdata",
        "Syncplay on ajan tasalla",
        "Syncplay est a jour",
        "Syncplay e aggiornato",
        "O Syncplay esta atualizado",
        "Syncplay guncel",
        "Syncplay obnovlen do poslednei versii",
        "Syncplay yi shi zuixin banben",
        "Syncplayneun choesin sangtaeimnida",
    )
}

pub(super) fn localized_syncplay_update_available_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    localized_literal(
        language,
        "A new version of Syncplay is available. Do you want to visit the release page?",
        "Eine neue Version von Syncplay ist verfuegbar. Moechten Sie die Release-Seite besuchen?",
        "Hay una nueva version de Syncplay disponible. Desea visitar la pagina de lanzamiento?",
        "Nova versio de Syncplay disponeblas. Chu vi volas viziti la eldonan paghon?",
        "Uusi Syncplay-versio on saatavilla. Haluatko avata julkaisusivun?",
        "Une nouvelle version de Syncplay est disponible. Voulez-vous visiter la page de publication?",
        "E disponibile una nuova versione di Syncplay. Vuoi visitare la pagina di rilascio?",
        "Uma nova versao do Syncplay esta disponivel. Deseja visitar a pagina de lancamento?",
        "Syncplay'in yeni bir surumu mevcut. Surum sayfasini ziyaret etmek ister misiniz?",
        "Dostupna novaia versiia Syncplay. Otkryt stranicu vypuska?",
        "You xin de Syncplay banben ke yong. Yao fangwen fabu yemian ma?",
        "Syncplay-ui saeroun beojeoni isseumnida. baepo peijireul bangmunhasigesseumnikka?",
    )
}

pub(super) fn localized_update_check_failed_message_legacy_compatible(
    language: Option<&str>,
    version: &str,
) -> String {
    localized_literal(
        language,
        "Could not automatically check whether Syncplay {} is up to date. Want to visit https://syncplay.pl/ to manually check for updates?",
        "Es konnte nicht automatisch geprueft werden, ob Syncplay {} aktuell ist. Moechten Sie https://syncplay.pl/ besuchen, um manuell nach Updates zu suchen?",
        "No se pudo comprobar automaticamente si Syncplay {} esta actualizado. Desea visitar https://syncplay.pl/ para comprobar manualmente si hay actualizaciones?",
        "Ne eblis auxtomate kontroli chu Syncplay {} estas gxisdata. Chu vi volas viziti https://syncplay.pl/ por mane kontroli gxisdatigojn?",
        "Ei voitu tarkistaa automaattisesti, onko Syncplay {} ajan tasalla. Haluatko kayda osoitteessa https://syncplay.pl/ tarkistaaksesi paivitykset manuaalisesti?",
        "Impossible de verifier automatiquement si Syncplay {} est a jour. Voulez-vous visiter https://syncplay.pl/ pour verifier manuellement les mises a jour?",
        "Impossibile verificare automaticamente se Syncplay {} e aggiornato. Vuoi visitare https://syncplay.pl/ per controllare manualmente gli aggiornamenti?",
        "Nao foi possivel verificar automaticamente se o Syncplay {} esta atualizado. Deseja visitar https://syncplay.pl/ para verificar atualizacoes manualmente?",
        "Syncplay {}'in guncel olup olmadigi otomatik olarak denetlenemedi. Guncellemeleri elle kontrol etmek icin https://syncplay.pl/ adresini ziyaret etmek ister misiniz?",
        "Ne udalos avtomaticheski proverit, obnovlen li Syncplay {}. Hotite pereiti na https://syncplay.pl/ dlia ruchnoi proverki obnovlenii?",
        "Wu fa zidong jiancha Syncplay {} shifou wei zuixin banben. Yao fangwen https://syncplay.pl/ shoudong jiancha gengxin ma?",
        "Syncplay {}ga choesin beojeoninji jadongeuro hwaginhal su eopseotseumnida. susdong-euro eobdeiteureul hwaginhagi wihae https://syncplay.pl/ reul bangmunhasigesseumnikka?",
    )
    .replace("{}", version)
}

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

fn localize_exact_message(message: &str, language: Option<&str>) -> Option<String> {
    let localized = match strip_period(message) {
        "About dialog opened" => localized_literal(
            language,
            "About dialog opened",
            "Info-Dialog geoeffnet",
            "Dialogo Acerca de abierto",
            "Pri-dialogo malfermita",
            "Tietoja-valintaikkuna avattu",
            "Boite A propos ouverte",
            "Finestra Informazioni aperta",
            "Janela Sobre aberta",
            "Hakkinda iletisi acildi",
            "Okno O programme otkryto",
            "Guanyu duihua kuang yi dakai",
            "jeongbo daehwa sangjaga yeollim",
        ),
        "Help requested" => localized_literal(
            language,
            "Help requested",
            "Hilfe angefordert",
            "Ayuda solicitada",
            "Helpo petita",
            "Ohje pyydetty",
            "Aide demandee",
            "Aiuto richiesto",
            "Ajuda solicitada",
            "Yardim istendi",
            "Zaprosena spravka",
            "Yi qingqiu bangzhu",
            "doum-eul yocheongham",
        ),
        "Help opened" => localized_literal(
            language,
            "Help opened",
            "Hilfe geoeffnet",
            "Ayuda abierta",
            "Helpo malfermita",
            "Ohje avattu",
            "Aide ouverte",
            "Aiuto aperto",
            "Ajuda aberta",
            "Yardim acildi",
            "Spravka otkryta",
            "Bangzhu yi dakai",
            "doum-i yeollim",
        ),
        "Open media file requested" => localized_literal(
            language,
            "Open media file requested",
            "Medienoeffnung angefordert",
            "Solicitud para abrir archivo multimedia",
            "Peto por malfermi amaskomunikan dosieron",
            "Mediatiedoston avaus pyydetty",
            "Ouverture de fichier media demandee",
            "Richiesta apertura file multimediale",
            "Solicitada abertura de arquivo de midia",
            "Ortam dosyasi acma istegi",
            "Zapros na otkrytie mediafaila",
            "Qingqiu dakai meiti wenjian",
            "midieo pail yeolgi yocheong",
        ),
        "Media search opened" => localized_literal(
            language,
            "Media search opened",
            "Mediensuche geoeffnet",
            "Busqueda de medios abierta",
            "Serco de amaskomunikiloj malfermita",
            "Mediavalu avattu",
            "Recherche media ouverte",
            "Ricerca media aperta",
            "Pesquisa de midia aberta",
            "Medya aramasi acildi",
            "Poisk media otkryt",
            "Meiti sousuo yi dakai",
            "midieo geomsa-ga yeollim",
        ),
        "Public server browser opened" => localized_literal(
            language,
            "Public server browser opened",
            "Browser fuer oeffentliche Server geoeffnet",
            "Navegador de servidores publicos abierto",
            "Foliumilo de publikaj serviloj malfermita",
            "Julkisten palvelinten selain avattu",
            "Navigateur des serveurs publics ouvert",
            "Browser dei server pubblici aperto",
            "Navegador de servidores publicos aberto",
            "Genel sunucu tarayicisi acildi",
            "Obozrevatel publichnykh serverov otkryt",
            "Gonggong fuwuqi liulanqi yi dakai",
            "gonggae seobeo beuraujeoga yeollim",
        ),
        "Exit requested" => localized_literal(
            language,
            "Exit requested",
            "Beenden angefordert",
            "Salida solicitada",
            "Eliro petita",
            "Poistuminen pyydetty",
            "Sortie demandee",
            "Uscita richiesta",
            "Saida solicitada",
            "Cikis istendi",
            "Vykhod zaproshen",
            "Qingqiu tuichu",
            "jonglyo yocheong",
        ),
        "Shared playlist opened" => localized_literal(
            language,
            "Shared playlist opened",
            "Playlist-Aktionen geoeffnet",
            "Acciones de lista de reproduccion abiertas",
            "Agoj de ludlisto malfermitaj",
            "Soittolistatoiminnot avattu",
            "Actions de playlist ouvertes",
            "Azioni playlist aperte",
            "Acoes da playlist abertas",
            "Calma listesi eylemleri acildi",
            "Deistviia plejlista otkryty",
            "Bofang liebiao dongzuo yi dakai",
            "jaesaeng mongnog jageobi yeollim",
        ),
        "Trusted domains opened" => localized_literal(
            language,
            "Trusted domains opened",
            "Vertrauenswuerdige Domains geoeffnet",
            "Dominios de confianza abiertos",
            "Fidindaj domajnoj malfermitaj",
            "Luotetut toimialueet avattu",
            "Domaines de confiance ouverts",
            "Domini attendibili aperti",
            "Dominios confiaveis abertos",
            "Guvenilen alan adlari acildi",
            "Doverennye domeny otkryty",
            "Yishou ren de yuming yi dakai",
            "sinroehal su inneun domein-i yeollim",
        ),
        "TLS certificate prompt opened" => localized_literal(
            language,
            "TLS certificate prompt opened",
            "TLS-Zertifikatabfrage geoeffnet",
            "Solicitud de certificado TLS abierta",
            "TLS-atestila avizo malfermita",
            "TLS-varmennedialogi avattu",
            "Invite de certificat TLS ouverte",
            "Prompt certificato TLS aperto",
            "Aviso de certificado TLS aberto",
            "TLS sertifika istemi acildi",
            "Zapros sertifikata TLS otkryt",
            "TLS zhengshu tishi yi dakai",
            "TLS jeungmyeongseo chajang-i yeollim",
        ),
        "Update notice opened" => localized_literal(
            language,
            "Update notice opened",
            "Update-Hinweis geoeffnet",
            "Aviso de actualizacion abierto",
            "Gxisdiga avizo malfermita",
            "Paivitysilmoitus avattu",
            "Avis de mise a jour ouvert",
            "Avviso di aggiornamento aperto",
            "Aviso de atualizacao aberto",
            "Guncelleme bildirimi acildi",
            "Uvedomlenie ob obnovlenii otkryto",
            "Gengxin tongzhi yi dakai",
            "eobdeiteu annae-ga yeollim",
        ),
        "Update notice dismissed" => localized_literal(
            language,
            "Update notice dismissed",
            "Update-Hinweis geschlossen",
            "Aviso de actualizacion descartado",
            "Gxisdiga avizo fermita",
            "Paivitysilmoitus suljettu",
            "Avis de mise a jour ferme",
            "Avviso di aggiornamento chiuso",
            "Aviso de atualizacao dispensado",
            "Guncelleme bildirimi kapatildi",
            "Uvedomlenie ob obnovlenii zakryto",
            "Gengxin tongzhi yi guanbi",
            "eobdeiteu annae-reul dadat-seumnida",
        ),
        "Chat sent" => localized_literal(
            language,
            "Chat sent",
            "Chat gesendet",
            "Chat enviado",
            "Babilejo sendita",
            "Keskusteluviesti lahetetty",
            "Message envoye",
            "Chat inviato",
            "Chat enviado",
            "Sohbet gonderildi",
            "Chat soobshchenie otpravleno",
            "Liaotian yi fasong",
            "chaeting-i jeonsongdoeeotseumnida",
        ),
        "Chat send canceled" => localized_literal(
            language,
            "Chat send canceled",
            "Chat-Senden abgebrochen",
            "Envio de chat cancelado",
            "Sendado de babilejo nuligita",
            "Keskusteluviestin lahetys peruttu",
            "Envoi du message annule",
            "Invio chat annullato",
            "Envio do chat cancelado",
            "Sohbet gonderimi iptal edildi",
            "Otpravka chat-soobshcheniia otmenena",
            "Liaotian fasong yi quxiao",
            "chaeting jeonsong-i chwiso-doeeotseumnida",
        ),
        "Session disconnected" => localized_literal(
            language,
            "Session disconnected",
            "Sitzung getrennt",
            "Sesion desconectada",
            "Sesio malkonektita",
            "Istunto katkaistu",
            "Session deconnectee",
            "Sessione disconnessa",
            "Sessao desconectada",
            "Oturum baglantisi kesildi",
            "Sessiia otkliuchena",
            "Huihua yi duankai",
            "sesiuni yeongyeol-i kkeun-eojyeosseumnida",
        ),
        "Session reconnected" => localized_literal(
            language,
            "Session reconnected",
            "Sitzung erneut verbunden",
            "Sesion reconectada",
            "Sesio rekonektita",
            "Istunto yhdistetty uudelleen",
            "Session reconnectee",
            "Sessione riconnessa",
            "Sessao reconectada",
            "Oturum yeniden baglandi",
            "Sessiia povtorno podkliuchena",
            "Huihua yi chongxin lianjie",
            "sesiuni dasi yeongyeoldoeeotseumnida",
        ),
        "Restoring session state" => localized_literal(
            language,
            "Restoring session state",
            "Sitzungsstatus wird wiederhergestellt",
            "Restaurando el estado de la sesion",
            "Restarigante sesian staton",
            "Palautetaan istunnon tila",
            "Restauration de l'etat de session",
            "Ripristino dello stato della sessione",
            "Restaurando o estado da sessao",
            "Oturum durumu geri yukleniyor",
            "Vosstanovlenie sostoianiia sessii",
            "Zhengzai huifu huihua zhuangtai",
            "sesiun sangtaereul bokguhaneun jung",
        ),
        "Restoring shared playlist state" => localized_literal(
            language,
            "Restoring shared playlist state",
            "Status der gemeinsamen Playlist wird wiederhergestellt",
            "Restaurando el estado de la lista compartida",
            "Restarigante komunan ludlistan staton",
            "Palautetaan jaetun soittolistan tila",
            "Restauration de l'etat de la playlist partagee",
            "Ripristino dello stato della playlist condivisa",
            "Restaurando o estado da playlist compartilhada",
            "Paylasilan oynatma listesi durumu geri yukleniyor",
            "Vosstanovlenie sostoianiia obshchego plejlista",
            "Zhengzai huifu gongxiang bofang liebiao zhuangtai",
            "gongyu jaesaeng mongnog sangtaereul bokguhaneun jung",
        ),
        "Session state correction recovery cooldown ended" => localized_literal(
            language,
            "Session state correction recovery cooldown ended",
            "Abklingzeit fuer Sitzungsstatus-Korrektur beendet",
            "Finalizo el enfriamiento de recuperacion de la correccion del estado de la sesion",
            "Reakira malvarmigo de sesia stata korekto finigxis",
            "Istunnon tilakorjauksen palautumisviive paattyi",
            "La temporisation de reprise de la correction de l'etat de session est terminee",
            "Il cooldown di recupero della correzione dello stato della sessione e terminato",
            "O cooldown de recuperacao da correcao do estado da sessao terminou",
            "Oturum durumu duzeltme toparlanma beklemesi bitti",
            "Pauza vosstanovleniia ispravleniia sostoianiia sessii zavershena",
            "Huihua zhuangtai xiuzheng huifu lengque yi jieshu",
            "sesiun sangtae gyujeong hoebok daegi sigani kkeutnatseumnida",
        ),
        "Configured server connect canceled" => localized_literal(
            language,
            "Configured server connect canceled",
            "Verbindung zum konfigurierten Server abgebrochen",
            "Conexion al servidor configurado cancelada",
            "Konekto al agordita servilo nuligita",
            "Yhteys maaritettyyn palvelimeen peruttu",
            "Connexion au serveur configure annulee",
            "Connessione al server configurato annullata",
            "Conexao ao servidor configurado cancelada",
            "Yapilandirilmis sunucu baglantisi iptal edildi",
            "Podkliuchenie k nastroennomu serveru otmeneno",
            "Yulian peizhi fuwuqi lianjie yi quxiao",
            "seoljeongdoen seobeo yeongyeoli chwiso-doeeotseumnida",
        ),
        "Disconnecting the current session" => localized_literal(
            language,
            "Disconnecting the current session",
            "Aktuelle Sitzung wird getrennt",
            "Desconectando la sesion actual",
            "Malkonektante la nunan sesion",
            "Katkaistaan nykyinen istunto",
            "Deconnexion de la session actuelle",
            "Disconnessione della sessione corrente",
            "Desconectando a sessao atual",
            "Gecerli oturumun baglantisi kesiliyor",
            "Tekushchaia sessiia otkliuchaetsia",
            "Zhengzai duankai dangqian huihua",
            "hyeonjae sesiuneul kkeunneun jung",
        ),
        "Session disconnect canceled" => localized_literal(
            language,
            "Session disconnect canceled",
            "Trennen der Sitzung abgebrochen",
            "Desconexion de sesion cancelada",
            "Malkonekto de sesio nuligita",
            "Istunnon katkaisu peruttu",
            "Deconnexion de la session annulee",
            "Disconnessione della sessione annullata",
            "Desconexao da sessao cancelada",
            "Oturum baglantisini kesme iptal edildi",
            "Otkliuchenie sessii otmeneno",
            "Huihua duankai yi quxiao",
            "sesiun yeongyeol haejega chwiso-doeeotseumnida",
        ),
        "Public server connect canceled" => localized_literal(
            language,
            "Public server connect canceled",
            "Verbindung zum oeffentlichen Server abgebrochen",
            "Conexion al servidor publico cancelada",
            "Konekto al publika servilo nuligita",
            "Yhteys julkiseen palvelimeen peruttu",
            "Connexion au serveur public annulee",
            "Connessione al server pubblico annullata",
            "Conexao ao servidor publico cancelada",
            "Genel sunucu baglantisi iptal edildi",
            "Podkliuchenie k publichnomu serveru otmeneno",
            "Gonggong fuwuqi lianjie yi quxiao",
            "gonggae seobeo yeongyeoli chwiso-doeeotseumnida",
        ),
        "Refreshing public servers" => localized_literal(
            language,
            "Refreshing public servers",
            "Oeffentliche Server werden aktualisiert",
            "Actualizando servidores publicos",
            "Refresxigante publikajn servilojn",
            "Paivitetaan julkisia palvelimia",
            "Actualisation des serveurs publics",
            "Aggiornamento dei server pubblici",
            "Atualizando servidores publicos",
            "Genel sunucular yenileniyor",
            "Obnovlenie spiska publichnykh serverov",
            "Zhengzai shuaxin gonggong fuwuqi",
            "gonggae seobeoreul saerobogeohaneun jung",
        ),
        "Public server refresh canceled" => localized_literal(
            language,
            "Public server refresh canceled",
            "Aktualisierung oeffentlicher Server abgebrochen",
            "Actualizacion de servidores publicos cancelada",
            "Refresxigo de publikaj serviloj nuligita",
            "Julkisten palvelinten paivitys peruttu",
            "Actualisation des serveurs publics annulee",
            "Aggiornamento dei server pubblici annullato",
            "Atualizacao de servidores publicos cancelada",
            "Genel sunucu yenilemesi iptal edildi",
            "Obnovlenie publichnykh serverov otmeneno",
            "Gonggong fuwuqi shuaxin yi quxiao",
            "gonggae seobeo saerobogiga chwiso-doeeotseumnida",
        ),
        "Missing-media search started" => localized_literal(
            language,
            "Missing-media search started",
            "Suche nach fehlenden Medien gestartet",
            "Busqueda de medios faltantes iniciada",
            "Serco de mankantaj amaskomunikiloj komencita",
            "Puuttuvan median haku aloitettu",
            "Recherche de medias manquants demarree",
            "Ricerca dei media mancanti avviata",
            "Pesquisa de midia ausente iniciada",
            "Eksik medya aramasi baslatildi",
            "Poisk otsutstvuiushchego media zapushchen",
            "Que shi meiti sousuo yi kaishi",
            "nujlagdoen midieo geomsa-ga sijakdoeeotseumnida",
        ),
        "Missing-media search canceled" => localized_literal(
            language,
            "Missing-media search canceled",
            "Suche nach fehlenden Medien abgebrochen",
            "Busqueda de medios faltantes cancelada",
            "Serco de mankantaj amaskomunikiloj nuligita",
            "Puuttuvan median haku peruttu",
            "Recherche de medias manquants annulee",
            "Ricerca dei media mancanti annullata",
            "Pesquisa de midia ausente cancelada",
            "Eksik medya aramasi iptal edildi",
            "Poisk otsutstvuiushchego media otmenen",
            "Que shi meiti sousuo yi quxiao",
            "nujlagdoen midieo geomsa-ga chwiso-doeeotseumnida",
        ),
        "Playback paused" => localized_literal(
            language,
            "Playback paused",
            "Wiedergabe pausiert",
            "Reproduccion pausada",
            "Ludado pausigita",
            "Toisto pysaytetty",
            "Lecture mise en pause",
            "Riproduzione in pausa",
            "Reproducao pausada",
            "Oynatma duraklatildi",
            "Vosproizvedenie postavleno na pauzu",
            "Bofang yi zanting",
            "jaesaeng-i ilsi jeongjidoeeotseumnida",
        ),
        "Playback resumed" => localized_literal(
            language,
            "Playback resumed",
            "Wiedergabe fortgesetzt",
            "Reproduccion reanudada",
            "Ludado rekomencita",
            "Toisto jatkettu",
            "Lecture reprise",
            "Riproduzione ripresa",
            "Reproducao retomada",
            "Oynatma devam etti",
            "Vosproizvedenie vozobnovleno",
            "Bofang yi jixu",
            "jaesaeng-i jaegaedoeeotseumnida",
        ),
        "Autoplay enabled" => localized_literal(
            language,
            "Autoplay enabled",
            "Autoplay aktiviert",
            "Reproduccion automatica activada",
            "Axtomata ludado sxaltita",
            "Automaattitoisto kaytossa",
            "Lecture automatique activee",
            "Riproduzione automatica attivata",
            "Reproducao automatica ativada",
            "Otomatik oynatma etkin",
            "Avtovosproizvedenie vkliucheno",
            "Zidong bofang yi qiyong",
            "jadong jaesaeng-i hwalseonghwa-doeeotseumnida",
        ),
        "Autoplay disabled" => localized_literal(
            language,
            "Autoplay disabled",
            "Autoplay deaktiviert",
            "Reproduccion automatica desactivada",
            "Axtomata ludado malsxaltita",
            "Automaattitoisto poistettu kaytosta",
            "Lecture automatique desactivee",
            "Riproduzione automatica disattivata",
            "Reproducao automatica desativada",
            "Otomatik oynatma devre disi",
            "Avtovosproizvedenie vykliucheno",
            "Zidong bofang yi tingyong",
            "jadong jaesaeng-i bihwalseonghwa-doeeotseumnida",
        ),
        "Configuration saved" => localized_literal(
            language,
            "Configuration saved",
            "Konfiguration gespeichert",
            "Configuracion guardada",
            "Agordo konservita",
            "Asetukset tallennettu",
            "Configuration enregistree",
            "Configurazione salvata",
            "Configuracao salva",
            "Yapilandirma kaydedildi",
            "Konfiguratsiia sokhranena",
            "Peizhi yi baocun",
            "guseongi jeojangdoeeotseumnida",
        ),
        "Syncplay media directories have been updated" => localized_literal(
            language,
            "Syncplay media directories have been updated",
            "Syncplay-Medienverzeichnisse wurden aktualisiert",
            "Los directorios multimedia de Syncplay se han actualizado",
            "La amaskomunikilaj dosierujoj de Syncplay estis gxisdatigitaj",
            "Syncplayn mediakansiot on paivitetty",
            "Les repertoires multimedias Syncplay ont ete mis a jour",
            "Le directory multimediali di Syncplay sono state aggiornate",
            "Os diretorios de midia do Syncplay foram atualizados",
            "Syncplay medya dizinleri guncellendi",
            "Katalogi media Syncplay obnovleny",
            "Syncplay meiti mulu yi gengxin",
            "Syncplay midieo diregtoriga eobdeiteu-doeeotseumnida",
        ),
        "Missing media search completed: no match found" => localized_literal(
            language,
            "Missing media search completed: no match found",
            "Suche nach fehlenden Medien abgeschlossen: kein Treffer",
            "Busqueda de medios faltantes completada: no se encontro coincidencia",
            "Serco de mankantaj amaskomunikiloj finita: neniu kongruo trovita",
            "Puuttuvan median haku valmis: ei osumaa",
            "Recherche de medias manquants terminee : aucune correspondance",
            "Ricerca dei media mancanti completata: nessuna corrispondenza trovata",
            "Pesquisa de midia ausente concluida: nenhuma correspondencia encontrada",
            "Eksik medya aramasi tamamlandi: eslesme bulunamadi",
            "Poisk otsutstvuiushchego media zavershen: sovpadenii ne naideno",
            "Que shi meiti sousuo yi wancheng: wei zhaodao pipei xiang",
            "nujlagdoen midieo geomsa-ga wallyodoeeotjiman ilchi haneun hangmogi eopseumnida",
        ),
        "Syncplay is up to date" => localized_syncplay_uptodate_message_legacy_compatible(language),
        "A new version of Syncplay is available. Do you want to visit the release page?" => {
            localized_syncplay_update_available_message_legacy_compatible(language)
        }
        "An update notice is available for this client build" => {
            localized_update_notice_available_message_legacy_compatible(language)
        }
        "Dismiss it here or trigger another update check from the same modal" => {
            localized_update_dismiss_hint_line_legacy_compatible(language)
        }
        _ => return None,
    };
    Some(with_original_terminal_period(message, localized))
}

fn localize_pattern_message(message: &str, language: Option<&str>) -> Option<String> {
    if let Some(label) = message
        .strip_prefix("Public server selected: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {label}",
            localized_literal(
                language,
                "Public server selected",
                "Oeffentlicher Server gewaehlt",
                "Servidor publico seleccionado",
                "Publika servilo elektita",
                "Julkinen palvelin valittu",
                "Serveur public selectionne",
                "Server pubblico selezionato",
                "Servidor publico selecionado",
                "Genel sunucu secildi",
                "Publichnyi server vybran",
                "Yi xuanze gonggong fuwuqi",
                "gonggae seobeoga seontaekdoeeotseumnida"
            )
        )));
    }
    if let Some(label) = message
        .strip_prefix("Custom public server added: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {label}",
            localized_literal(
                language,
                "Custom public server added",
                "Benutzerdefinierter oeffentlicher Server hinzugefuegt",
                "Servidor publico personalizado agregado",
                "Propra publika servilo aldonita",
                "Mukautettu julkinen palvelin lisatty",
                "Serveur public personnalise ajoute",
                "Server pubblico personalizzato aggiunto",
                "Servidor publico personalizado adicionado",
                "Ozel genel sunucu eklendi",
                "Dobavlen polzovatelskii publichnyi server",
                "Yi tianjia zidingyi gonggong fuwuqi",
                "sayongja jijeong gonggae seobeoga chugadoeeotseumnida"
            )
        )));
    }
    if let Some(count) = message
        .strip_prefix("Public servers refreshed: ")
        .and_then(|value| value.strip_suffix(" entries."))
    {
        return Some(with_terminal_period(&format!(
            "{}: {count} {}",
            localized_literal(
                language,
                "Public servers refreshed",
                "Oeffentliche Server aktualisiert",
                "Servidores publicos actualizados",
                "Publikaj serviloj refresxigitaj",
                "Julkiset palvelimet paivitetty",
                "Serveurs publics actualises",
                "Server pubblici aggiornati",
                "Servidores publicos atualizados",
                "Genel sunucular yenilendi",
                "Publichnye servery obnovleny",
                "Gonggong fuwuqi yi shuaxin",
                "gonggae seobeoga saerobogidoeeotseumnida"
            ),
            localized_literal(
                language,
                "entries",
                "Eintraege",
                "entradas",
                "eroj",
                "merkintaa",
                "elements",
                "elementi",
                "entradas",
                "girdi",
                "zapisey",
                "xiang",
                "hangmog"
            ),
        )));
    }
    if let Some(address) = message
        .strip_prefix("Connecting to configured server: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {address}",
            localized_literal(
                language,
                "Connecting to configured server",
                "Verbinde mit konfiguriertem Server",
                "Conectando al servidor configurado",
                "Konektante al agordita servilo",
                "Yhdistetaan maaritettyyn palvelimeen",
                "Connexion au serveur configure",
                "Connessione al server configurato",
                "Conectando ao servidor configurado",
                "Yapilandirilmis sunucuya baglaniliyor",
                "Podkliuchenie k nastroennomu serveru",
                "Zhengzai lianjie yulian peizhi fuwuqi",
                "seoljeongdoen seobeoe yeongyeol jung"
            )
        )));
    }
    if let Some(address) = message
        .strip_prefix("Connected to configured server: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {address}",
            localized_literal(
                language,
                "Connected to configured server",
                "Mit konfiguriertem Server verbunden",
                "Conectado al servidor configurado",
                "Konektita al agordita servilo",
                "Yhdistetty maaritettyyn palvelimeen",
                "Connecte au serveur configure",
                "Connesso al server configurato",
                "Conectado ao servidor configurado",
                "Yapilandirilmis sunucuya baglanildi",
                "Podkliucheno k nastroennomu serveru",
                "Yi lianjie yulian peizhi fuwuqi",
                "seoljeongdoen seobeoe yeongyeoldoeeotseumnida"
            )
        )));
    }
    if let Some(label) = message
        .strip_prefix("Connecting to public server: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {label}",
            localized_literal(
                language,
                "Connecting to public server",
                "Verbinde mit oeffentlichem Server",
                "Conectando al servidor publico",
                "Konektante al publika servilo",
                "Yhdistetaan julkiseen palvelimeen",
                "Connexion au serveur public",
                "Connessione al server pubblico",
                "Conectando ao servidor publico",
                "Genel sunucuya baglaniliyor",
                "Podkliuchenie k publichnomu serveru",
                "Zhengzai lianjie gonggong fuwuqi",
                "gonggae seobeoe yeongyeol jung"
            )
        )));
    }
    if let Some(label) = message
        .strip_prefix("Connected to public server: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {label}",
            localized_literal(
                language,
                "Connected to public server",
                "Mit oeffentlichem Server verbunden",
                "Conectado al servidor publico",
                "Konektita al publika servilo",
                "Yhdistetty julkiseen palvelimeen",
                "Connecte au serveur public",
                "Connesso al server pubblico",
                "Conectado ao servidor publico",
                "Genel sunucuya baglanildi",
                "Podkliucheno k publichnomu serveru",
                "Yi lianjie gonggong fuwuqi",
                "gonggae seobeoe yeongyeoldoeeotseumnida"
            )
        )));
    }
    if let Some(path) = message
        .strip_prefix("Media search directory selected: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {path}",
            localized_literal(
                language,
                "Media search directory selected",
                "Mediensuchverzeichnis gewaehlt",
                "Directorio de busqueda de medios seleccionado",
                "Elektita serc-dosierujo por amaskomunikiloj",
                "Mediahaun hakemisto valittu",
                "Repertoire de recherche media selectionne",
                "Directory di ricerca media selezionata",
                "Diretorio de pesquisa de midia selecionado",
                "Medya arama dizini secildi",
                "Vybran katalog poiska media",
                "Yi xuanze meiti sousuo mulu",
                "midieo geomsa diregtoriga seontaekdoeeotseumnida"
            )
        )));
    }
    if let Some(path) = message
        .strip_prefix("Media search directory added: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {path}",
            localized_literal(
                language,
                "Media search directory added",
                "Mediensuchverzeichnis hinzugefuegt",
                "Directorio de busqueda de medios agregado",
                "Serc-dosierujo por amaskomunikiloj aldonita",
                "Mediahaun hakemisto lisatty",
                "Repertoire de recherche media ajoute",
                "Directory di ricerca media aggiunta",
                "Diretorio de pesquisa de midia adicionado",
                "Medya arama dizini eklendi",
                "Dobavlen katalog poiska media",
                "Yi tianjia meiti sousuo mulu",
                "midieo geomsa diregtoriga chugadoeeotseumnida"
            )
        )));
    }
    if let Some(path) = message
        .strip_prefix("Missing media found: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {path}",
            localized_literal(
                language,
                "Missing media found",
                "Fehlende Medien gefunden",
                "Medio faltante encontrado",
                "Mankanta amaskomunikilo trovita",
                "Puuttuva media loytyi",
                "Media manquant trouve",
                "Media mancante trovato",
                "Midia ausente encontrada",
                "Eksik medya bulundu",
                "Otsutstvuiushchee media naideno",
                "Zhaodao que shi meiti",
                "nujlagdoen midieoreul chajasseumnida"
            )
        )));
    }
    if let Some(domain) = message
        .strip_prefix("Trusted domain already present: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {domain}",
            localized_literal(
                language,
                "Trusted domain already present",
                "Vertrauenswuerdige Domain bereits vorhanden",
                "El dominio de confianza ya existe",
                "Fidinda domajno jam cxeestas",
                "Luotettu toimialue on jo olemassa",
                "Le domaine de confiance est deja present",
                "Il dominio attendibile e gia presente",
                "Dominio confiavel ja presente",
                "Guvenilen alan adi zaten mevcut",
                "Doverennyi domen uzhe prisutstvuet",
                "Yishou ren de yuming yi cunzai",
                "sinroehal su inneun domein-i imi jonjaeham"
            )
        )));
    }
    if let Some(domain) = message
        .strip_prefix("Trusted domain added: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {domain}",
            localized_literal(
                language,
                "Trusted domain added",
                "Vertrauenswuerdige Domain hinzugefuegt",
                "Dominio de confianza agregado",
                "Fidinda domajno aldonita",
                "Luotettu toimialue lisatty",
                "Domaine de confiance ajoute",
                "Dominio attendibile aggiunto",
                "Dominio confiavel adicionado",
                "Guvenilen alan adi eklendi",
                "Dobavlen doverennyi domen",
                "Yi tianjia yishou ren de yuming",
                "sinroehal su inneun domein-i chugadoeeotseumnida"
            )
        )));
    }
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
    if let Some(rest) = message.strip_prefix("Reconnect attempt ")
        && let Some((attempt, delay)) = rest
            .strip_suffix(" seconds.")
            .and_then(|value| value.split_once(" in "))
    {
        return Some(with_terminal_period(&format!(
            "{} {attempt} {} {delay} {}",
            localized_literal(
                language,
                "Reconnect attempt",
                "Wiederverbindungsversuch",
                "Intento de reconexion",
                "Rekonektprovo",
                "Uudelleenyhdistamisyritys",
                "Tentative de reconnexion",
                "Tentativo di riconnessione",
                "Tentativa de reconexao",
                "Yeniden baglanma denemesi",
                "Popytka povtornogo podkliucheniia",
                "Chongxin lianjie changshi",
                "dasi yeongyeol si-do"
            ),
            localized_literal(
                language, "in", "in", "en", "post", "kuluttua", "dans", "tra", "em", "icinde",
                "cherez", "zai", "hu"
            ),
            localized_literal(
                language,
                "seconds",
                "Sekunden",
                "segundos",
                "sekundoj",
                "sekunnin kuluttua",
                "secondes",
                "secondi",
                "segundos",
                "saniye",
                "sekund",
                "miao",
                "cho"
            ),
        )));
    }
    if let Some(room) = message
        .strip_prefix("Requesting controller access for ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{} {room}",
            localized_literal(
                language,
                "Requesting controller access for",
                "Fordere Controller-Zugriff an fuer",
                "Solicitando acceso de controlador para",
                "Petante regantan aliron por",
                "Pyydetaan ohjaajaoikeuksia kohteelle",
                "Demande d'acces controleur pour",
                "Richiesta accesso controller per",
                "Solicitando acesso de controlador para",
                "Denetleyici erisimi isteniyor:",
                "Zaprashivaetsia dostup kontrolera dlia",
                "Zhengzai qingqiu kongzhizhe quanxian gei",
                "kontroller jeobgeun gwoneul yocheongham:"
            )
        )));
    }
    if let Some(message) =
        strip_period(message).strip_prefix("Session state restore mismatch detected (")
        && let Some(seconds) = message.strip_suffix(" seconds)")
    {
        return Some(with_terminal_period(&format!(
            "{} ({seconds} {})",
            localized_literal(
                language,
                "Session state restore mismatch detected",
                "Abweichung bei Wiederherstellung des Sitzungsstatus erkannt",
                "Se detecto una discrepancia al restaurar el estado de la sesion",
                "Malkongruo dum restarigo de sesia stato detektita",
                "Istunnon tilan palautuspoikkeama havaittu",
                "Ecart detecte lors de la restauration de l'etat de session",
                "Rilevata una discrepanza nel ripristino dello stato della sessione",
                "Divergencia detectada na restauracao do estado da sessao",
                "Oturum durumu geri yuklemesinde uyumsuzluk algilandi",
                "Obnaruzheno nesootvetstvie pri vosstanovlenii sostoianiia sessii",
                "Jiance dao huihua zhuangtai huifu chayi",
                "sesiun sangtae bogwon bulilchi-ga gamjidoeeotseumnida"
            ),
            localized_literal(
                language, "seconds", "Sekunden", "segundos", "sekundoj", "sekuntia", "secondes",
                "secondi", "segundos", "saniye", "sekund", "miao", "cho"
            )
        )));
    }
    if let Some(message) = strip_period(message).strip_prefix("Session state correction retry ")
        && let Some((attempts, ticks)) = message.split_once(" scheduled after ")
        && let Some(ticks) = ticks.strip_suffix(" ticks")
    {
        return Some(with_terminal_period(&format!(
            "{} {attempts} {} {ticks} {}",
            localized_literal(
                language,
                "Session state correction retry",
                "Sitzungsstatus-Korrekturversuch",
                "Reintento de correccion del estado de la sesion",
                "Sesia stata korekta reprovo",
                "Istunnon tilakorjauksen uusintayritys",
                "Nouvelle tentative de correction de l'etat de session",
                "Nuovo tentativo di correzione dello stato della sessione",
                "Nova tentativa de correcao do estado da sessao",
                "Oturum durumu duzeltme yeniden denemesi",
                "Povtornaya popytka ispravleniia sostoianiia sessii",
                "Huihua zhuangtai xiuzheng chongshi",
                "sesiun sangtae gyujeong jaesido"
            ),
            localized_literal(
                language,
                "scheduled after",
                "geplant nach",
                "programado tras",
                "planita post",
                "ajastettu",
                "planifie apres",
                "pianificato dopo",
                "agendada apos",
                "su kadar sonra planlandi",
                "zaplanirano cherez",
                "jiang zai ci hou anpai",
                "daeum ihu yeyakdoeeotseumnida"
            ),
            localized_literal(
                language,
                "ticks",
                "Ticks",
                "ticks",
                "tikoj",
                "tikkujen jalkeen",
                "ticks",
                "tick",
                "ticks",
                "tik",
                "tikov",
                "ci",
                "tik"
            )
        )));
    }
    if let Some(attempts) = message
        .strip_prefix("Session state correction exhausted after ")
        .and_then(|value| value.strip_suffix(" attempts."))
    {
        return Some(with_terminal_period(&format!(
            "{} {attempts} {}",
            localized_literal(
                language,
                "Session state correction exhausted after",
                "Sitzungsstatus-Korrektur ausgeschopft nach",
                "La correccion del estado de la sesion se agoto tras",
                "Sesia stata korekto eluzita post",
                "Istunnon tilakorjaus loppui jalkeen",
                "Correction de l'etat de session epuisee apres",
                "Correzione dello stato della sessione esaurita dopo",
                "Correcao do estado da sessao esgotada apos",
                "Oturum durumu duzeltmesi su denemeden sonra tukendi",
                "Ispravlenie sostoianiia sessii ischerpano posle",
                "Huihua zhuangtai xiuzheng zai ci hou yongjin",
                "sesiun sangtae gyujeongi daeum hu modu sojindoeeotseumnida"
            ),
            localized_literal(
                language,
                "attempts",
                "Versuchen",
                "intentos",
                "provoj",
                "yrityksen jalkeen",
                "tentatives",
                "tentativi",
                "tentativas",
                "deneme",
                "popytok",
                "ci changshi",
                "beon si-do"
            )
        )));
    }
    if let Some(cycles) = message
        .strip_prefix("Session state correction disabled after ")
        .and_then(|value| value.strip_suffix(" mismatch cycles."))
    {
        return Some(with_terminal_period(&format!(
            "{} {cycles} {}",
            localized_literal(
                language,
                "Session state correction disabled after",
                "Sitzungsstatus-Korrektur deaktiviert nach",
                "Correccion del estado de la sesion desactivada tras",
                "Sesia stata korekto malaktivigita post",
                "Istunnon tilakorjaus poistettiin kaytosta jalkeen",
                "Correction de l'etat de session desactivee apres",
                "Correzione dello stato della sessione disattivata dopo",
                "Correcao do estado da sessao desativada apos",
                "Oturum durumu duzeltmesi su kadar sonra devre disi birakildi",
                "Ispravlenie sostoianiia sessii otkliucheno posle",
                "Huihua zhuangtai xiuzheng zai ci hou bei tingyong",
                "sesiun sangtae gyujeongi daeum hu bihwalseonghwa-doeeotseumnida"
            ),
            localized_literal(
                language,
                "mismatch cycles",
                "Abweichungszyklen",
                "ciclos de discrepancia",
                "malkongruaj cikloj",
                "poikkeamajakson jalkeen",
                "cycles de divergence",
                "cicli di discrepanza",
                "ciclos de divergencia",
                "uyumsuzluk dongusu",
                "tsiklov nesootvetstviia",
                "ci bu pipei zhouqi",
                "bulilchi ju-gi"
            )
        )));
    }
    if let Some(cycles) = message
        .strip_prefix("Session state correction recovery cooldown active for ")
        .and_then(|value| value.strip_suffix(" more reconnect cycles."))
    {
        return Some(with_terminal_period(&format!(
            "{} {cycles} {}",
            localized_literal(
                language,
                "Session state correction recovery cooldown active for",
                "Abklingzeit der Sitzungsstatus-Korrektur aktiv fuer",
                "Enfriamiento de recuperacion de la correccion del estado de la sesion activo durante",
                "Reakira malvarmigo de sesia stata korekto aktivas por",
                "Istunnon tilakorjauksen palautumisen viive on aktiivinen viela",
                "Temporisation de reprise de la correction de l'etat de session active pendant",
                "Cooldown di recupero della correzione dello stato della sessione attivo per",
                "Cooldown de recuperacao da correcao do estado da sessao ativo por",
                "Oturum durumu duzeltme toparlanma beklemesi su kadar daha etkin",
                "Pauza vosstanovleniia ispravleniia sostoianiia sessii aktivna eshche na",
                "Huihua zhuangtai xiuzheng huifu lengque zai ci qijian reng huoyue",
                "sesiun sangtae gyujeong hoebok daegi sigani daeum dongan hwalseongimnida"
            ),
            localized_literal(
                language,
                "more reconnect cycles",
                "weitere Wiederverbindungszyklen",
                "ciclos mas de reconexion",
                "pliajn rekonektajn ciklojn",
                "uudelleenyhdistamiskertaa",
                "cycles de reconnexion supplementaires",
                "ulteriori cicli di riconnessione",
                "mais ciclos de reconexao",
                "daha fazla yeniden baglanma dongusu",
                "tsiklov povtornogo podkliucheniia",
                "geng duo chongxin lianjie zhouqi",
                "deo maneun dasi yeongyeol ju-gi"
            )
        )));
    }
    if let Some(count) = message
        .strip_prefix("Autoplay minimum users set to ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{} {count}",
            localized_literal(
                language,
                "Autoplay minimum users set to",
                "Autoplay-Mindestanzahl gesetzt auf",
                "Usuarios minimos de reproduccion automatica fijados en",
                "Minimumaj uzantoj por auxtoludo agorditaj al",
                "Automaattitoiston vahimmaiskayttajamaara asetettu arvoon",
                "Nombre minimum d'utilisateurs pour la lecture automatique defini a",
                "Utenti minimi per l'autoplay impostati su",
                "Usuarios minimos do autoplay definidos para",
                "Otomatik oynatma minimum kullanici sayisi sunu oldu",
                "Minimalnoe chislo polzovatelei dlia avtovosproizvedeniia ustanovleno na",
                "Zidong bofang zui shao yonghu shu yi shewei",
                "jadong jaesaeng choeso sayongja suga daeum-euro seoljeongdoeeotseumnida"
            )
        )));
    }
    if let Some(count) = message.strip_prefix("Autoplay in ")
        && let Some((seconds_left, ready_count)) = count
            .strip_suffix(" ready users.")
            .and_then(|value| value.split_once(" seconds with "))
    {
        return Some(with_terminal_period(&format!(
            "{} {seconds_left} {} {ready_count} {}",
            localized_literal(
                language,
                "Autoplay in",
                "Autoplay in",
                "Reproduccion automatica en",
                "Auxtoludo post",
                "Automaattitoisto alkaa",
                "Lecture automatique dans",
                "Autoplay tra",
                "Autoplay em",
                "Otomatik oynatma",
                "Avtovosproizvedenie cherez",
                "Zidong bofang zai",
                "jadong jaesaeng"
            ),
            localized_literal(
                language,
                "seconds with",
                "Sekunden mit",
                "segundos con",
                "sekundoj kun",
                "sekunnin kuluttua ja",
                "secondes avec",
                "secondi con",
                "segundos com",
                "saniye sonra",
                "sekund s",
                "miao hou bing you",
                "cho hu"
            ),
            localized_literal(
                language,
                "ready users",
                "bereiten Benutzern",
                "usuarios listos",
                "pretaj uzantoj",
                "valmista kayttajaa",
                "utilisateurs prets",
                "utenti pronti",
                "usuarios prontos",
                "hazir kullanici",
                "gotovykh polzovatelei",
                "zhunbei yonghu",
                "junbi sayongja"
            ),
        )));
    }
    if let Some(room) = message
        .strip_prefix("Controlled room created: ")
        .and_then(|value| value.strip_suffix('.'))
    {
        return Some(with_terminal_period(&format!(
            "{}: {room}",
            localized_literal(
                language,
                "Controlled room created",
                "Kontrollierter Raum erstellt",
                "Sala controlada creada",
                "Regata cxambro kreita",
                "Hallittu huone luotu",
                "Salle controlee creee",
                "Stanza controllata creata",
                "Sala controlada criada",
                "Denetimli oda olusturuldu",
                "Upravliaemaia komnata sozdana",
                "Yi chuangjian shoukong fangjian",
                "jeeo bang-i saengseongdoeeotseumnida"
            )
        )));
    }
    if let Some(rest) = message
        .strip_prefix("Created controlled room ")
        .and_then(|value| value.strip_suffix('.'))
        && let Some((room, tail)) = rest.split_once(" with password ")
    {
        return Some(with_terminal_period(&format!(
            "{} {room} {} {tail}",
            localized_literal(
                language,
                "Created controlled room",
                "Kontrollierten Raum erstellt",
                "Sala controlada creada",
                "Kreis regatan cxambron",
                "Loi hallitun huoneen",
                "Salle controlee creee",
                "Stanza controllata creata",
                "Sala controlada criada",
                "Denetimli oda olusturuldu",
                "Sozdana upravliaemaia komnata",
                "Yi chuangjian shoukong fangjian",
                "jeeo bang-eul saengseonghaetseumnida"
            ),
            localized_literal(
                language,
                "with password",
                "mit Passwort",
                "con contrasena",
                "kun pasvorto",
                "salasanalla",
                "avec le mot de passe",
                "con password",
                "com senha",
                "parolayla",
                "s parolem",
                "mima wei",
                "bi-milbeonho:"
            )
        )));
    }
    if let Some(count) = message
        .strip_prefix("Room history updated: ")
        .and_then(|value| value.strip_suffix(" entries."))
    {
        return Some(with_terminal_period(&format!(
            "{}: {count} {}",
            localized_literal(
                language,
                "Room history updated",
                "Raumverlauf aktualisiert",
                "Historial de salas actualizado",
                "Cxambra historio gxisdatigita",
                "Huonehistoria paivitetty",
                "Historique des salles mis a jour",
                "Cronologia stanze aggiornata",
                "Historico de salas atualizado",
                "Oda gecmisi guncellendi",
                "Istoriia komnat obnovlena",
                "Fangjian lishi yi gengxin",
                "bang girag-i eobdeiteu-doeeotseumnida"
            ),
            localized_literal(
                language,
                "entries",
                "Eintraege",
                "entradas",
                "eroj",
                "merkintaa",
                "elements",
                "elementi",
                "entradas",
                "girdi",
                "zapisey",
                "xiang",
                "hangmog"
            ),
        )));
    }
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

fn localize_generic_error_message(message: &str, language: Option<&str>) -> Option<String> {
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

pub(super) fn localize_gui_runtime_message_legacy_compatible(
    message: &str,
    language: Option<&str>,
) -> String {
    let language = Some(normalized_runtime_language_tag_or_default(language));
    localize_exact_message(message, language)
        .or_else(|| localize_pattern_message(message, language))
        .or_else(|| localize_generic_error_message(message, language))
        .unwrap_or_else(|| message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        localize_gui_runtime_message_legacy_compatible,
        localized_public_server_list_failed_message_legacy_compatible,
        localized_syncplay_uptodate_message_legacy_compatible,
        localized_update_check_failed_message_legacy_compatible,
    };

    #[test]
    fn service_messages_use_selected_language() {
        assert_eq!(
            localized_syncplay_uptodate_message_legacy_compatible(Some("fr")),
            "Syncplay est a jour"
        );
        assert_eq!(
            localized_public_server_list_failed_message_legacy_compatible(Some("de")),
            "Die Liste der oeffentlichen Server konnte nicht geladen werden. Bitte besuchen Sie https://www.syncplay.pl/ in Ihrem Browser."
        );
        assert!(
            localized_update_check_failed_message_legacy_compatible(Some("fr"), "1.7.5")
                .contains("Syncplay 1.7.5")
        );
    }

    #[test]
    fn localized_runtime_message_translates_public_server_and_update_strings() {
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible(
                "Public servers refreshed: 2 entries.",
                Some("fr"),
            ),
            "Serveurs publics actualises: 2 elements."
        );
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible("Syncplay is up to date.", Some("fr")),
            "Syncplay est a jour."
        );
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible(
                "No public server refresh is currently in progress.",
                Some("fr"),
            ),
            "Aucune operation active pour \"public server refresh\"."
        );
    }

    #[test]
    fn localized_runtime_message_preserves_english_wording_and_localizes_runtime_patterns() {
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible(
                "No public server refresh is currently in progress.",
                Some("en"),
            ),
            "No public server refresh is currently in progress."
        );
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible(
                "Requesting controller access for +room:ABCDEF123456.",
                Some("fr"),
            ),
            "Demande d'acces controleur pour +room:ABCDEF123456."
        );
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible("Session reconnected.", Some("fr"),),
            "Session reconnectee."
        );
        assert_eq!(
            localize_gui_runtime_message_legacy_compatible(
                "Session state restore mismatch detected (2.500 seconds).",
                Some("fr"),
            ),
            "Ecart detecte lors de la restauration de l'etat de session (2.500 secondes)."
        );
    }
}
