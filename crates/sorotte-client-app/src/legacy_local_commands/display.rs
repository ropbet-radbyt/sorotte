use sorotte_client_core::ClientSession;

use super::types::{LocalInputCommandErrorKind, PlannedLocalInputDispatch};

const PLAYLIST_EMPTY_MESSAGE_LEGACY: &str = "Playlist is currently empty.";
const PLAYLIST_INVALID_INDEX_ERROR_LEGACY: &str = "Invalid playlist index";
const QUEUE_MISSING_FILE_ERROR_LEGACY: &str = "No file/url given";
const UNKNOWN_COMMAND_MESSAGE_LEGACY: &str = "Unrecognized command";
const PROJECT_URL_LEGACY: &str = "https://syncplay.pl/";

pub fn localized_local_input_error_message_legacy_compatible(
    error_kind: LocalInputCommandErrorKind,
    language: Option<&str>,
) -> &'static str {
    match (error_kind, language) {
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("de")) => {
            "Ungueltiger Playlist-Index"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("es")) => {
            "Indice de lista de reproduccion no valido"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("eo")) => {
            "Nevalida ludlista indekso"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("fi")) => {
            "Virheellinen soittolistaindeksi"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("fr")) => {
            "Indice de playlist non valide"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("it")) => {
            "Indice della playlist non valido"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("pt_PT" | "pt_BR")) => {
            "Indice de playlist invalido"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("tr")) => {
            "Gecersiz oynatma listesi indeksi"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("ru")) => {
            "Nedopustimyi indeks spiska vosproizvedeniia"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("zh_CN")) => {
            "Wuxiao de bofang liebiao suoyin"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, Some("ko")) => {
            "Yuhyo haji an-eun jaesaeng moglog indeks"
        }
        (LocalInputCommandErrorKind::PlaylistInvalidIndex, _) => {
            PLAYLIST_INVALID_INDEX_ERROR_LEGACY
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("de")) => "Keine Datei/URL angegeben",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("es")) => {
            "No se proporciono archivo/url"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("eo")) => "Neniu dosiero/url donita",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("fi")) => {
            "Tiedostoa/url-osoitetta ei annettu"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("fr")) => "Aucun fichier/url fourni",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("it")) => "Nessun file/url fornito",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("pt_PT" | "pt_BR")) => {
            "Nenhum arquivo/url informado"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, Some("tr")) => "Dosya/url verilmedi",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("ru")) => "Fail/url ne ukazan",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("zh_CN")) => "Wei tigong wenjian/url",
        (LocalInputCommandErrorKind::QueueMissingFile, Some("ko")) => {
            "Pail/url-i jegongdoeji anassseumnida"
        }
        (LocalInputCommandErrorKind::QueueMissingFile, _) => QUEUE_MISSING_FILE_ERROR_LEGACY,
    }
}

fn localized_error_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "FEHLER",
        Some("es") => "ERROR",
        Some("eo") => "ERARO",
        Some("fi") => "VIRHE",
        Some("fr") => "ERREUR",
        Some("it") => "ERRORE",
        Some("pt_PT" | "pt_BR") => "ERRO",
        Some("tr") => "HATA",
        Some("ru") => "OSHIBKA",
        Some("zh_CN") => "CUOWU",
        Some("ko") => "OREU",
        _ => "ERROR",
    }
}

pub fn local_input_error_output_line_legacy_compatible(
    error_kind: LocalInputCommandErrorKind,
    language: Option<&str>,
) -> String {
    format!(
        "{}:\t{}",
        localized_error_prefix_legacy_compatible(language),
        localized_local_input_error_message_legacy_compatible(error_kind, language)
    )
}

pub(crate) fn localized_unknown_command_message_legacy_compatible(
    language: Option<&str>,
) -> &'static str {
    match language {
        Some("de") => "Unbekannter Befehl",
        Some("es") => "Comando no reconocido",
        Some("eo") => "Nekonata komando",
        Some("fi") => "Tuntematon komento",
        Some("fr") => "Commande non reconnue",
        Some("it") => "Comando non riconosciuto",
        Some("pt_PT" | "pt_BR") => "Comando nao reconhecido",
        Some("tr") => "Taninmayan komut",
        Some("ru") => "Neopoznannaia komanda",
        Some("zh_CN") => "Wei shibie de mingling",
        Some("ko") => "Insikhal su eomneun myeongryeong",
        _ => UNKNOWN_COMMAND_MESSAGE_LEGACY,
    }
}

fn localized_local_command_help_heading_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Verfuegbare Befehle:",
        Some("es") => "Comandos disponibles:",
        Some("eo") => "Doneblaj ordonoj:",
        Some("fi") => "Kaytettavissa olevat komennot:",
        Some("fr") => "Commandes disponibles:",
        Some("it") => "Comandi disponibili:",
        Some("pt_PT" | "pt_BR") => "Comandos disponiveis:",
        Some("tr") => "Kullanilabilir komutlar:",
        Some("ru") => "Dostupnye komandy:",
        Some("zh_CN") => "Ke yong mingling:",
        Some("ko") => "Sayong ganeunghan myeongryeong:",
        _ => "Available commands:",
    }
}

fn local_command_help_command_lines_legacy_compatible() -> &'static [&'static str] {
    &[
        "\tr [name] - change room",
        "\tl - show user list",
        "\tu - undo last seek",
        "\tp - toggle pause",
        "\t[s][+-]time - seek to the given value of time, if + or - is not specified it's absolute time in seconds or min:sec",
        "\to[+-]duration - offset local playback by the given duration (in seconds or min:sec) from the server seek position - this is a deprecated feature",
        "\th - this help",
        "\tt - toggles whether you are ready to watch or not",
        "\tsr [name] - sets user as ready",
        "\tsn [name] - sets user as not ready",
        "\tc [name] - create managed room using name of current room",
        "\ta [password] - authenticate as room operator with operator password",
        "\tch [message] - send a chat message in a room",
        "\tqa [file/url] - add file or url to bottom of playlist",
        "\tqas [file/url] - add file or url to bottom of playlist and select it",
        "\tql - show the current playlist",
        "\tqs [index] - select given entry in the playlist",
        "\tqn - select next entry in the playlist",
        "\tqd [index] - delete the given entry from the playlist",
        "\tshuffleremainingplaylist - shuffle remaining playlist entries",
        "\tshuffleentireplaylist - shuffle entire playlist and reset index to 1",
        "\tundoplaylist - undo last playlist change",
    ]
}

fn localized_local_command_help_command_lines_legacy_compatible(
    language: Option<&str>,
) -> &'static [&'static str] {
    match language {
        Some("de") => &[
            "\tr [name] - Raum wechseln",
            "\tl - Benutzerliste anzeigen",
            "\tu - letzten Suchsprung rueckgaengig machen",
            "\tp - Pause umschalten",
            "\t[s][+-]time - zur angegebenen Zeit springen; ohne + oder - ist dies eine absolute Zeit in Sekunden oder min:sec",
            "\to[+-]duration - lokale Wiedergabe relativ zur Server-Position um die angegebene Dauer verschieben (in Sekunden oder min:sec) - dies ist eine veraltete Funktion",
            "\th - diese Hilfe",
            "\tt - Bereitschaft zum Zuschauen umschalten",
            "\tsr [name] - Benutzer auf bereit setzen",
            "\tsn [name] - Benutzer auf nicht bereit setzen",
            "\tc [name] - verwalteten Raum aus dem Namen des aktuellen Raums erstellen",
            "\ta [password] - als Raumoperator mit Operator-Passwort authentifizieren",
            "\tch [message] - Chat-Nachricht im Raum senden",
            "\tqa [file/url] - Datei oder URL ans Ende der Playlist anhaengen",
            "\tqas [file/url] - Datei oder URL ans Ende der Playlist anhaengen und auswaehlen",
            "\tql - aktuelle Playlist anzeigen",
            "\tqs [index] - angegebenen Eintrag in der Playlist auswaehlen",
            "\tqn - naechsten Eintrag in der Playlist auswaehlen",
            "\tqd [index] - angegebenen Eintrag aus der Playlist loeschen",
            "\tshuffleremainingplaylist - verbleibende Playlist-Eintraege mischen",
            "\tshuffleentireplaylist - gesamte Playlist mischen und Index auf 1 zuruecksetzen",
            "\tundoplaylist - letzte Playlist-Aenderung rueckgaengig machen",
        ],
        Some("es") => &[
            "\tr [name] - cambiar de sala",
            "\tl - mostrar lista de usuarios",
            "\tu - deshacer la ultima busqueda",
            "\tp - alternar pausa",
            "\t[s][+-]time - buscar al valor de tiempo indicado; si no se especifica + o -, es tiempo absoluto en segundos o min:sec",
            "\to[+-]duration - desplazar la reproduccion local segun la duracion indicada (en segundos o min:sec) respecto a la posicion del servidor - esta es una funcion obsoleta",
            "\th - esta ayuda",
            "\tt - alterna si estas listo para ver o no",
            "\tsr [name] - marcar usuario como listo",
            "\tsn [name] - marcar usuario como no listo",
            "\tc [name] - crear sala gestionada usando el nombre de la sala actual",
            "\ta [password] - autenticarse como operador de la sala con la contrasena de operador",
            "\tch [message] - enviar un mensaje de chat en una sala",
            "\tqa [file/url] - agregar archivo o url al final de la lista de reproduccion",
            "\tqas [file/url] - agregar archivo o url al final de la lista y seleccionarlo",
            "\tql - mostrar la lista de reproduccion actual",
            "\tqs [index] - seleccionar la entrada indicada en la lista de reproduccion",
            "\tqn - seleccionar la siguiente entrada de la lista de reproduccion",
            "\tqd [index] - eliminar la entrada indicada de la lista de reproduccion",
            "\tshuffleremainingplaylist - mezclar las entradas restantes de la lista de reproduccion",
            "\tshuffleentireplaylist - mezclar toda la lista de reproduccion y restablecer el indice a 1",
            "\tundoplaylist - deshacer el ultimo cambio de la lista de reproduccion",
        ],
        Some("eo") => &[
            "\tr [name] - sxangxi cxambron",
            "\tl - montri uzantoliston",
            "\tu - malfari lastan sercxon",
            "\tp - sxalti pauzon",
            "\t[s][+-]time - salti al la donita tempo; sen + au - gxi estas absoluta tempo en sekundoj au min:sec",
            "\to[+-]duration - sxovi lokan reprodukton per la donita dauro (en sekundoj au min:sec) disde la servila pozicio - tio estas malrekomendita trajto",
            "\th - tiu helpo",
            "\tt - sxaltas cxu vi pretas spekti au ne",
            "\tsr [name] - marki uzanton preta",
            "\tsn [name] - marki uzanton ne preta",
            "\tc [name] - krei administratan cxambron uzante la nomon de la nuna cxambro",
            "\ta [password] - autentikigi kiel cxambro-operatoro per operatora pasvorto",
            "\tch [message] - sendi babilejan mesagxon en cxambro",
            "\tqa [file/url] - aldoni dosieron au url-on al la fino de la ludlisto",
            "\tqas [file/url] - aldoni dosieron au url-on al la fino de la ludlisto kaj elekti gxin",
            "\tql - montri la nunan ludliston",
            "\tqs [index] - elekti la donitan eron en la ludlisto",
            "\tqn - elekti la sekvan eron en la ludlisto",
            "\tqd [index] - forigi la donitan eron el la ludlisto",
            "\tshuffleremainingplaylist - miksi la restantajn ludlistajn erojn",
            "\tshuffleentireplaylist - miksi la tutan ludliston kaj reagordi la indekson al 1",
            "\tundoplaylist - malfari la lastan ludlistan sxangxon",
        ],
        Some("fi") => &[
            "\tr [name] - vaihda huonetta",
            "\tl - nayta kayttajalista",
            "\tu - kumoa viimeisin haku",
            "\tp - vaihda tauko",
            "\t[s][+-]time - siirry annettuun aikaan; ilman + tai - kyseessa on absoluuttinen aika sekunteina tai min:sec",
            "\to[+-]duration - siirra paikallista toistoa annetulla kestolla (sekunteina tai min:sec) palvelimen hakusijaintiin nahden - tama on vanhentunut ominaisuus",
            "\th - tama ohje",
            "\tt - vaihtaa oletko valmis katsomaan vai et",
            "\tsr [name] - merkitse kayttaja valmiiksi",
            "\tsn [name] - merkitse kayttaja ei-valmiiksi",
            "\tc [name] - luo hallittu huone nykyisen huoneen nimen perusteella",
            "\ta [password] - tunnistaudu huoneen operaattoriksi operaattorin salasanalla",
            "\tch [message] - laheta chat-viesti huoneessa",
            "\tqa [file/url] - lisaa tiedosto tai url soittolistan loppuun",
            "\tqas [file/url] - lisaa tiedosto tai url soittolistan loppuun ja valitse se",
            "\tql - nayta nykyinen soittolista",
            "\tqs [index] - valitse annettu merkinta soittolistasta",
            "\tqn - valitse seuraava merkinta soittolistasta",
            "\tqd [index] - poista annettu merkinta soittolistasta",
            "\tshuffleremainingplaylist - sekoita jaljella olevat soittolistan merkinnat",
            "\tshuffleentireplaylist - sekoita koko soittolista ja nollaa indeksi arvoon 1",
            "\tundoplaylist - kumoa viimeisin soittolistan muutos",
        ],
        Some("fr") => &[
            "\tr [name] - changer de salle",
            "\tl - afficher la liste des utilisateurs",
            "\tu - annuler le dernier seek",
            "\tp - basculer la pause",
            "\t[s][+-]time - aller a la valeur de temps indiquee ; sans + ou -, c'est un temps absolu en secondes ou min:sec",
            "\to[+-]duration - decaler la lecture locale de la duree indiquee (en secondes ou min:sec) par rapport a la position du serveur - c'est une fonctionnalite obsolete",
            "\th - cette aide",
            "\tt - basculer votre etat pret/pas pret",
            "\tsr [name] - definir l'utilisateur comme pret",
            "\tsn [name] - definir l'utilisateur comme non pret",
            "\tc [name] - creer une salle geree a partir du nom de la salle actuelle",
            "\ta [password] - s'authentifier comme operateur de salle avec le mot de passe operateur",
            "\tch [message] - envoyer un message de chat dans une salle",
            "\tqa [file/url] - ajouter un fichier ou une url en bas de la playlist",
            "\tqas [file/url] - ajouter un fichier ou une url en bas de la playlist et le selectionner",
            "\tql - afficher la playlist actuelle",
            "\tqs [index] - selectionner l'entree indiquee dans la playlist",
            "\tqn - selectionner l'entree suivante dans la playlist",
            "\tqd [index] - supprimer l'entree indiquee de la playlist",
            "\tshuffleremainingplaylist - melanger les entrees restantes de la playlist",
            "\tshuffleentireplaylist - melanger toute la playlist et reinitialiser l'index a 1",
            "\tundoplaylist - annuler la derniere modification de la playlist",
        ],
        Some("it") => &[
            "\tr [name] - cambia stanza",
            "\tl - mostra elenco utenti",
            "\tu - annulla l'ultimo seek",
            "\tp - attiva/disattiva pausa",
            "\t[s][+-]time - vai al valore di tempo indicato; se + o - non e specificato, e tempo assoluto in secondi o min:sec",
            "\to[+-]duration - sposta la riproduzione locale della durata indicata (in secondi o min:sec) rispetto alla posizione del server - questa e una funzione deprecata",
            "\th - questo aiuto",
            "\tt - alterna se sei pronto a guardare oppure no",
            "\tsr [name] - imposta utente come pronto",
            "\tsn [name] - imposta utente come non pronto",
            "\tc [name] - crea una stanza gestita usando il nome della stanza corrente",
            "\ta [password] - autenticati come operatore della stanza con la password operatore",
            "\tch [message] - invia un messaggio di chat in una stanza",
            "\tqa [file/url] - aggiungi file o url in fondo alla playlist",
            "\tqas [file/url] - aggiungi file o url in fondo alla playlist e selezionalo",
            "\tql - mostra la playlist corrente",
            "\tqs [index] - seleziona la voce indicata nella playlist",
            "\tqn - seleziona la voce successiva nella playlist",
            "\tqd [index] - elimina la voce indicata dalla playlist",
            "\tshuffleremainingplaylist - mescola le voci rimanenti della playlist",
            "\tshuffleentireplaylist - mescola l'intera playlist e reimposta l'indice a 1",
            "\tundoplaylist - annulla l'ultima modifica della playlist",
        ],
        Some("pt_PT" | "pt_BR") => &[
            "\tr [name] - mudar de sala",
            "\tl - mostrar lista de usuarios",
            "\tu - desfazer a ultima busca",
            "\tp - alternar pausa",
            "\t[s][+-]time - buscar para o valor de tempo indicado; sem + ou -, e tempo absoluto em segundos ou min:sec",
            "\to[+-]duration - deslocar a reproducao local pela duracao indicada (em segundos ou min:sec) a partir da posicao do servidor - este e um recurso obsoleto",
            "\th - esta ajuda",
            "\tt - alterna se voce esta pronto para assistir ou nao",
            "\tsr [name] - marcar usuario como pronto",
            "\tsn [name] - marcar usuario como nao pronto",
            "\tc [name] - criar sala gerenciada usando o nome da sala atual",
            "\ta [password] - autenticar como operador da sala com a senha de operador",
            "\tch [message] - enviar uma mensagem de chat na sala",
            "\tqa [file/url] - adicionar arquivo ou url ao fim da playlist",
            "\tqas [file/url] - adicionar arquivo ou url ao fim da playlist e seleciona-lo",
            "\tql - mostrar a playlist atual",
            "\tqs [index] - selecionar a entrada indicada na playlist",
            "\tqn - selecionar a proxima entrada da playlist",
            "\tqd [index] - excluir a entrada indicada da playlist",
            "\tshuffleremainingplaylist - embaralhar as entradas restantes da playlist",
            "\tshuffleentireplaylist - embaralhar toda a playlist e redefinir o indice para 1",
            "\tundoplaylist - desfazer a ultima alteracao da playlist",
        ],
        Some("tr") => &[
            "\tr [name] - oda degistir",
            "\tl - kullanici listesini goster",
            "\tu - son aramayi geri al",
            "\tp - duraklatmayi degistir",
            "\t[s][+-]time - verilen zaman degerine git; + veya - yoksa bu saniye veya min:sec cinsinden mutlak zamandir",
            "\to[+-]duration - yerel oynatimi sunucu arama konumundan verilen sure kadar kaydir (saniye veya min:sec) - bu kullanimdan kalkan bir ozelliktir",
            "\th - bu yardim",
            "\tt - izlemeye hazir olup olmadiginizi degistirir",
            "\tsr [name] - kullaniciyi hazir olarak ayarla",
            "\tsn [name] - kullaniciyi hazir degil olarak ayarla",
            "\tc [name] - guncel oda adini kullanarak yonetilen oda olustur",
            "\ta [password] - operator parolasi ile oda operatoru olarak kimlik dogrula",
            "\tch [message] - odada sohbet mesaji gonder",
            "\tqa [file/url] - dosya veya url'yi oynatma listesinin sonuna ekle",
            "\tqas [file/url] - dosya veya url'yi oynatma listesinin sonuna ekle ve sec",
            "\tql - guncel oynatma listesini goster",
            "\tqs [index] - oynatma listesindeki belirtilen girdiyi sec",
            "\tqn - oynatma listesindeki sonraki girdiyi sec",
            "\tqd [index] - oynatma listesindeki belirtilen girdiyi sil",
            "\tshuffleremainingplaylist - kalan oynatma listesi girdilerini karistir",
            "\tshuffleentireplaylist - tum oynatma listesini karistir ve indeksi 1'e sifirla",
            "\tundoplaylist - son oynatma listesi degisikligini geri al",
        ],
        Some("ru") => &[
            "\tr [name] - smenit komnatu",
            "\tl - pokazat spisok polzovatelei",
            "\tu - otmenit poslednii peremot",
            "\tp - perekliuchit pausu",
            "\t[s][+-]time - pereiti k ukazannomu vremeni; bez + ili - eto absoliutnoe vremia v sekundakh ili min:sec",
            "\to[+-]duration - smestit lokalnoe vosproizvedenie na ukazannuiu dlinu (v sekundakh ili min:sec) otnositelno pozitsii servera - eto ustarevshaia funktsiia",
            "\th - eta spravka",
            "\tt - perekliuchaet vash status gotovnosti k prosmotru",
            "\tsr [name] - ustanovit polzovatelia gotovym",
            "\tsn [name] - ustanovit polzovatelia negotovym",
            "\tc [name] - sozdat upravliaemuiu komnatu s ispolzovaniem imeni tekushchei komnaty",
            "\ta [password] - avtorizovatsia kak operator komnaty s parolem operatora",
            "\tch [message] - otpravit soobshchenie chata v komnate",
            "\tqa [file/url] - dobavit fail ili url v konets spiska vosproizvedeniia",
            "\tqas [file/url] - dobavit fail ili url v konets spiska vosproizvedeniia i vybrat ego",
            "\tql - pokazat tekushchii spisok vosproizvedeniia",
            "\tqs [index] - vybrat ukazannyi element v spiske vosproizvedeniia",
            "\tqn - vybrat sleduiushchii element v spiske vosproizvedeniia",
            "\tqd [index] - udalit ukazannyi element iz spiska vosproizvedeniia",
            "\tshuffleremainingplaylist - peremeshat ostavshiesia elementy spiska vosproizvedeniia",
            "\tshuffleentireplaylist - peremeshat ves spisok vosproizvedeniia i sbrosit indeks na 1",
            "\tundoplaylist - otmenit poslednee izmenenie spiska vosproizvedeniia",
        ],
        Some("zh_CN") => &[
            "\tr [name] - qiehuan fangjian",
            "\tl - xianshi yonghu liebiao",
            "\tu - chexiao shangci tuidong",
            "\tp - qiehuan zan ting",
            "\t[s][+-]time - tiaozhuan dao geiding shijian; ru guo meiyou + huo -, ze wei miaoshu huo min:sec de juedui shijian",
            "\to[+-]duration - xiangdui fuwuqi weizhi an geiding shichang pianyi bendi bofang (danwei wei miao huo min:sec) - zhe shi yi ge yi feiqi de gongneng",
            "\th - ci bangzhu",
            "\tt - qiehuan ni shifou zhunbei hao guankan",
            "\tsr [name] - jiang yonghu she wei yi zhunbei",
            "\tsn [name] - jiang yonghu she wei wei zhunbei",
            "\tc [name] - yong dangqian fangjian mingcheng chuangjian guanli fangjian",
            "\ta [password] - shiyong fangjian guanliyuan mima jinxing shenfen yanzheng",
            "\tch [message] - zai fangjian zhong fasong liaotian xiaoxi",
            "\tqa [file/url] - jiang wenjian huo url tianjia dao bofang liebiao diduan",
            "\tqas [file/url] - jiang wenjian huo url tianjia dao bofang liebiao diduan bing xuanze ta",
            "\tql - xianshi dangqian bofang liebiao",
            "\tqs [index] - xuanze bofang liebiao zhong de zhiding tiao",
            "\tqn - xuanze bofang liebiao zhong de xia yi tiao",
            "\tqd [index] - shanchu bofang liebiao zhong de zhiding tiao",
            "\tshuffleremainingplaylist - suiji dapaisheng yu bofang liebiao tiao mu",
            "\tshuffleentireplaylist - suiji dapaisheng zhengge bofang liebiao bing jiang suoyin chongzhi wei 1",
            "\tundoplaylist - chexiao shangci bofang liebiao genggai",
        ],
        Some("ko") => &[
            "\tr [name] - bang byeongyeong",
            "\tl - sayongja moglog pyosi",
            "\tu - majimak sigeul chwiso",
            "\tp - ilsi jeongji jeonhwan",
            "\t[s][+-]time - jijeonghan sigan-euro idong; + na - ga eopseumyeon cho ttoneun min:sec-ui jeoldae sigan-ibnida",
            "\to[+-]duration - seobeo jompeu wichi-eseo jijeonghan siganmankeum lokal jaesaeng-eul omgim (cho ttoneun min:sec) - ibeoseun deo isang gwonjangdoeji anhneun gineung-imnida",
            "\th - i doum mal",
            "\tt - sicheong junbi sangtae jeonhwan",
            "\tsr [name] - sayongjareul junbi wanlyo-ro seoljeong",
            "\tsn [name] - sayongjareul junbi an doem-euro seoljeong",
            "\tc [name] - hyeonjae bang ireumeuro gwanli bang saengseong",
            "\ta [password] - unyeongja bimillo bang unyeongja-ro inyong",
            "\tch [message] - bang-eseo chaet mesiji bonaegi",
            "\tqa [file/url] - pail ttoneun url-eul jaesaeng moglog kkeut-e chuga",
            "\tqas [file/url] - pail ttoneun url-eul jaesaeng moglog kkeut-e chuga hago seontaek",
            "\tql - hyeonjae jaesaeng moglog pyosi",
            "\tqs [index] - jaesaeng moglog-eseo jijeonghan hangmog seontaek",
            "\tqn - jaesaeng moglog-ui daeum hangmog seontaek",
            "\tqd [index] - jaesaeng moglog-eseo jijeonghan hangmog sakje",
            "\tshuffleremainingplaylist - nam-eun jaesaeng moglog hangmog seokgi",
            "\tshuffleentireplaylist - jeonche jaesaeng moglog-eul seokgo indeks-reul 1lo chogiha",
            "\tundoplaylist - majimak jaesaeng moglog byeongyeong chwiso",
        ],
        _ => local_command_help_command_lines_legacy_compatible(),
    }
}

pub(crate) fn local_command_help_lines_legacy_compatible(language: Option<&str>) -> Vec<String> {
    let command_lines = localized_local_command_help_command_lines_legacy_compatible(language);
    let mut lines = Vec::with_capacity(command_lines.len() + 1);
    lines.push(localized_local_command_help_heading_legacy_compatible(language).to_owned());
    lines.extend(command_lines.iter().copied().map(str::to_owned));
    lines
}

fn localized_syncplay_version_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Sorotte-Version",
        Some("es") => "Version de Sorotte",
        Some("eo") => "Versio de Sorotte",
        Some("fi") => "Sorotte-versio",
        Some("fr") => "Version de Sorotte",
        Some("it") => "Versione di Sorotte",
        Some("pt_PT" | "pt_BR") => "Versao do Sorotte",
        Some("tr") => "Sorotte surumu",
        Some("ru") => "Versiia Sorotte",
        Some("zh_CN") => "Sorotte banben",
        Some("ko") => "Sorotte beojeon",
        _ => "Sorotte version",
    }
}

fn localized_more_info_prefix_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Mehr Informationen unter",
        Some("es") => "Mas informacion en",
        Some("eo") => "Pli da informoj che",
        Some("fi") => "Lisatietoja osoitteessa",
        Some("fr") => "Plus d'informations sur",
        Some("it") => "Maggiori informazioni su",
        Some("pt_PT" | "pt_BR") => "Mais informacoes em",
        Some("tr") => "Daha fazla bilgi",
        Some("ru") => "Bolshe informatsii na",
        Some("zh_CN") => "Geng duo xinxi qing fangwen",
        Some("ko") => "Deo maneun jeongbo",
        _ => "More info available at",
    }
}

pub(crate) fn local_command_help_footer_lines_legacy_compatible(
    language: Option<&str>,
    version: &str,
) -> [String; 2] {
    [
        format!(
            "{}: {version}",
            localized_syncplay_version_prefix_legacy_compatible(language)
        ),
        format!(
            "{}: {PROJECT_URL_LEGACY}",
            localized_more_info_prefix_legacy_compatible(language)
        ),
    ]
}

pub fn render_local_input_display_lines_legacy_compatible(
    dispatch: &PlannedLocalInputDispatch,
    session: &ClientSession,
    language: Option<&str>,
    version: &str,
) -> Option<Vec<String>> {
    match dispatch {
        PlannedLocalInputDispatch::Suppressed | PlannedLocalInputDispatch::Run(_) => None,
        PlannedLocalInputDispatch::EmitUnknownCommandHelp => {
            let mut lines = Vec::with_capacity(1 + 1 + 22 + 2);
            lines.push(localized_unknown_command_message_legacy_compatible(language).to_owned());
            lines.extend(local_command_help_lines_legacy_compatible(language));
            lines.extend(local_command_help_footer_lines_legacy_compatible(
                language, version,
            ));
            Some(lines)
        }
        PlannedLocalInputDispatch::EmitHelp => {
            let mut lines = local_command_help_lines_legacy_compatible(language);
            lines.extend(local_command_help_footer_lines_legacy_compatible(
                language, version,
            ));
            Some(lines)
        }
        PlannedLocalInputDispatch::EmitError(error_kind) => {
            Some(vec![local_input_error_output_line_legacy_compatible(
                *error_kind,
                language,
            )])
        }
        PlannedLocalInputDispatch::EmitPlaylist => {
            Some(vec![playlist_listing_message_localized_legacy_compatible(
                session, language,
            )])
        }
    }
}

pub fn localized_current_offset_message_legacy_compatible(
    offset_seconds: f64,
    language: Option<&str>,
) -> String {
    match language {
        Some("de") => format!("Aktueller Versatz: {offset_seconds} Sekunden"),
        Some("es") => format!("Desfase actual: {offset_seconds} segundos"),
        Some("eo") => format!("Nuna kompenso: {offset_seconds} sekundoj"),
        Some("fi") => format!("Nykyinen siirtyma: {offset_seconds} sekuntia"),
        Some("fr") => format!("Decalage actuel : {offset_seconds} secondes"),
        Some("it") => format!("Offset attuale: {offset_seconds} secondi"),
        Some("pt_PT" | "pt_BR") => format!("Deslocamento atual: {offset_seconds} segundos"),
        Some("tr") => format!("Guncel kaydirma: {offset_seconds} saniye"),
        Some("ru") => format!("Tekushchee smeshchenie: {offset_seconds} sekund"),
        Some("zh_CN") => format!("Dangqian pianyi: {offset_seconds} miao"),
        Some("ko") => format!("Hyeonjae opeuset: {offset_seconds} cho"),
        _ => format!("Current offset: {offset_seconds} seconds"),
    }
}

fn localized_playlist_empty_message_legacy_compatible(language: Option<&str>) -> &'static str {
    match language {
        Some("de") => "Playlist ist derzeit leer.",
        Some("es") => "La lista de reproduccion esta vacia.",
        Some("eo") => "La ludlisto estas malplena.",
        Some("fi") => "Soittolista on tyhja.",
        Some("fr") => "La playlist est actuellement vide.",
        Some("it") => "La playlist e attualmente vuota.",
        Some("pt_PT" | "pt_BR") => "A playlist esta vazia no momento.",
        Some("tr") => "Oynatma listesi su anda bos.",
        Some("ru") => "Spisok vosproizvedeniia seichas pust.",
        Some("zh_CN") => "Bofang liebiao muqian weikong.",
        Some("ko") => "Jaesaeng moglog-i hyeonjae bi-eo issseumnida.",
        _ => PLAYLIST_EMPTY_MESSAGE_LEGACY,
    }
}

pub fn playlist_listing_message_legacy_compatible(session: &ClientSession) -> String {
    let Some(playlist) = session.current_room_playlist() else {
        return PLAYLIST_EMPTY_MESSAGE_LEGACY.to_owned();
    };
    if playlist.files.is_empty() {
        return PLAYLIST_EMPTY_MESSAGE_LEGACY.to_owned();
    }

    let mut playlist_elements: Vec<String> = playlist
        .files
        .iter()
        .enumerate()
        .map(|(index, file_name)| format!("\t{}: {}", index + 1, file_name))
        .collect();
    if let Some(selected_index) = playlist.index.and_then(|index| usize::try_from(index).ok())
        && selected_index < playlist_elements.len()
    {
        playlist_elements[selected_index] = format!(" *{}", playlist_elements[selected_index]);
    }
    playlist_elements.join("\n")
}

pub fn playlist_listing_message_localized_legacy_compatible(
    session: &ClientSession,
    language: Option<&str>,
) -> String {
    let Some(playlist) = session.current_room_playlist() else {
        return localized_playlist_empty_message_legacy_compatible(language).to_owned();
    };
    if playlist.files.is_empty() {
        return localized_playlist_empty_message_legacy_compatible(language).to_owned();
    }
    playlist_listing_message_legacy_compatible(session)
}
