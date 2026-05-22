use super::super::*;

pub(super) fn localize_public_server_media_message(
    message: &str,
    language: Option<&str>,
) -> Option<String> {
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
    None
}
