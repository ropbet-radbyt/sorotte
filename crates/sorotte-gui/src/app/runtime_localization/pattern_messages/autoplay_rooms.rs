use super::super::*;

pub(super) fn localize_autoplay_rooms_message(
    message: &str,
    language: Option<&str>,
) -> Option<String> {
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
    None
}
