use super::super::*;

pub(super) fn localize_reconnect_state_message(
    message: &str,
    language: Option<&str>,
) -> Option<String> {
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
    None
}
