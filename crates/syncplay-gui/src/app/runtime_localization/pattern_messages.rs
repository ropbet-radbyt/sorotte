use super::*;

pub(super) fn localize_pattern_message(message: &str, language: Option<&str>) -> Option<String> {
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
