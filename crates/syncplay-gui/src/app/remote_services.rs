use std::{sync::OnceLock, time::Duration};

use reqwest::blocking::Client;
use serde_json::Value;
use syncplay_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;
use syncplay_client_app::app_boundary::persistence::parse_serialized_public_servers_list_legacy_compatible;
use syncplay_client_app::app_boundary::state::StoredClientSettingsMvp;

const LEGACY_SYNCPLAY_VERSION: &str = "1.7.5";
const LEGACY_SYNCPLAY_MILESTONE: &str = "Yoitsu";
const LEGACY_SYNCPLAY_RELEASE_NUMBER: &str = "116";
const LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS: u64 = 7 * 86_400;
const LEGACY_SYNCPLAY_VERSION_STATUS_UP_TO_DATE: &str = "uptodate";
const LEGACY_SYNCPLAY_VERSION_STATUS_UPDATE_AVAILABLE: &str = "updateavailale";
const SYNCPLAY_PUBLIC_SERVER_LIST_URL: &str = "https://syncplay.pl/listpublicservers";
const SYNCPLAY_UPDATE_URL: &str = "https://syncplay.pl/checkforupdate";
const SYNCPLAY_DOWNLOAD_URL: &str = "https://syncplay.pl/download/";
const SYNCPLAY_PUBLIC_SERVER_LIST_URL_ENV: &str = "SYNCPLAY_GUI_PUBLIC_SERVER_LIST_URL";
const SYNCPLAY_PUBLIC_SERVER_LIST_RESPONSE_ENV: &str = "SYNCPLAY_GUI_PUBLIC_SERVER_LIST_RESPONSE";
const SYNCPLAY_UPDATE_URL_ENV: &str = "SYNCPLAY_GUI_UPDATE_CHECK_URL";
const SYNCPLAY_UPDATE_CHECK_RESPONSE_ENV: &str = "SYNCPLAY_GUI_UPDATE_CHECK_RESPONSE";
static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LegacyUpdateCheckStatus {
    UpToDate,
    UpdateAvailable,
    Failed,
    Unknown(String),
}

impl LegacyUpdateCheckStatus {
    fn from_legacy_wire_value(value: &str) -> Self {
        match value.trim() {
            LEGACY_SYNCPLAY_VERSION_STATUS_UP_TO_DATE => Self::UpToDate,
            LEGACY_SYNCPLAY_VERSION_STATUS_UPDATE_AVAILABLE => Self::UpdateAvailable,
            "failed" => Self::Failed,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyUpdateCheckResult {
    pub(crate) status: LegacyUpdateCheckStatus,
    pub(crate) message: String,
    pub(crate) url: Option<String>,
    pub(crate) public_servers: Option<Vec<(String, String)>>,
    pub(crate) checked_at_utc: String,
    pub(crate) user_initiated: bool,
}

pub(crate) fn fetch_public_servers(
    language: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    if let Some(body) = env_response_override(SYNCPLAY_PUBLIC_SERVER_LIST_RESPONSE_ENV) {
        return parse_public_server_response(&body).map_err(|error| {
            format!(
                "{}\n-----\n{}",
                error,
                public_server_list_failed_message(language)
            )
        });
    }
    let url = std::env::var(SYNCPLAY_PUBLIC_SERVER_LIST_URL_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| SYNCPLAY_PUBLIC_SERVER_LIST_URL.to_owned());
    fetch_public_servers_from_url(&url, language).map_err(|error| {
        format!(
            "{}\n-----\n{}",
            error,
            public_server_list_failed_message(language)
        )
    })
}

fn fetch_public_servers_from_url(
    url: &str,
    language: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let language = normalized_language(language);
    let client = http_client()
        .map_err(|error| format!("failed to build public-server HTTP client: {error}"))?;

    let response = client
        .get(url)
        .query(&[
            ("version", LEGACY_SYNCPLAY_VERSION),
            ("milestone", LEGACY_SYNCPLAY_MILESTONE),
            ("release_number", LEGACY_SYNCPLAY_RELEASE_NUMBER),
            ("language", language),
        ])
        .send()
        .map_err(|error| format!("failed to load public server list: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "failed to load public server list: HTTP {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .map_err(|error| format!("failed to read public server list response: {error}"))?;
    parse_public_server_response(&body)
}

fn parse_public_server_response(body: &str) -> Result<Vec<(String, String)>, String> {
    let normalized = sanitize_wordpress_public_server_response(body);
    let Some(rows) = parse_serialized_public_servers_list_legacy_compatible(&normalized) else {
        return Err(
            "failed to parse public server list response from the Syncplay service".to_owned(),
        );
    };
    if rows.is_empty() {
        return Err(
            "failed to load public server list: the Syncplay service returned no servers"
                .to_owned(),
        );
    }
    Ok(rows)
}

pub(crate) fn check_for_updates(
    language: Option<&str>,
    user_initiated: bool,
) -> LegacyUpdateCheckResult {
    let checked_at_utc =
        legacy_utc_timestamp_string_legacy_compatible(std::time::SystemTime::now());
    if let Some(body) = env_response_override(SYNCPLAY_UPDATE_CHECK_RESPONSE_ENV) {
        return match parse_update_check_response(&body, language, user_initiated) {
            Ok(result) => LegacyUpdateCheckResult {
                checked_at_utc,
                user_initiated,
                ..result
            },
            Err(error) => LegacyUpdateCheckResult {
                status: LegacyUpdateCheckStatus::Failed,
                message: format!(
                    "{}\n-----\n{}",
                    error,
                    update_check_failed_notification_message(language)
                ),
                url: Some(SYNCPLAY_DOWNLOAD_URL.to_owned()),
                public_servers: None,
                checked_at_utc,
                user_initiated,
            },
        };
    }
    let url = std::env::var(SYNCPLAY_UPDATE_URL_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| SYNCPLAY_UPDATE_URL.to_owned());

    match fetch_update_check_result_from_url(&url, language, user_initiated) {
        Ok(result) => LegacyUpdateCheckResult {
            checked_at_utc,
            user_initiated,
            ..result
        },
        Err(error) => LegacyUpdateCheckResult {
            status: LegacyUpdateCheckStatus::Failed,
            message: format!(
                "{}\n-----\n{}",
                error,
                update_check_failed_notification_message(language)
            ),
            url: Some(SYNCPLAY_DOWNLOAD_URL.to_owned()),
            public_servers: None,
            checked_at_utc,
            user_initiated,
        },
    }
}

pub(crate) fn should_run_automatic_update_check(
    settings: Option<&StoredClientSettingsMvp>,
    now: std::time::SystemTime,
) -> bool {
    let Some(settings) = settings else {
        return false;
    };
    if settings.check_for_updates_automatically != Some(true) {
        return false;
    }
    let Some(last_checked) = settings
        .last_checked_for_updates
        .as_deref()
        .and_then(parse_legacy_utc_timestamp_legacy_compatible)
    else {
        return true;
    };

    now.duration_since(last_checked)
        .map(|elapsed| elapsed.as_secs() > LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS)
        .unwrap_or(false)
}

fn fetch_update_check_result_from_url(
    url: &str,
    language: Option<&str>,
    user_initiated: bool,
) -> Result<LegacyUpdateCheckResult, String> {
    let language = normalized_language(language);
    let client = http_client()
        .map_err(|error| format!("failed to build update-check HTTP client: {error}"))?;
    let response = client
        .get(url)
        .query(&[
            ("version", LEGACY_SYNCPLAY_VERSION),
            ("milestone", LEGACY_SYNCPLAY_MILESTONE),
            ("release_number", LEGACY_SYNCPLAY_RELEASE_NUMBER),
            ("language", language),
            ("platform", legacy_update_check_platform_name()),
            ("architecture", std::env::consts::ARCH),
            ("machine", std::env::consts::ARCH),
            (
                "userInitiated",
                if user_initiated { "True" } else { "False" },
            ),
        ])
        .send()
        .map_err(|error| format!("failed to run update check: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "failed to run update check: HTTP {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .map_err(|error| format!("failed to read update-check response: {error}"))?;
    parse_update_check_response(&body, Some(language), user_initiated)
}

fn parse_update_check_response(
    body: &str,
    language: Option<&str>,
    user_initiated: bool,
) -> Result<LegacyUpdateCheckResult, String> {
    let normalized = sanitize_wordpress_update_check_response(body);
    let parsed = serde_json::from_str::<Value>(&normalized)
        .map_err(|error| format!("failed to parse update-check response: {error}"))?;
    let raw_status = parsed
        .get("version-status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let status = LegacyUpdateCheckStatus::from_legacy_wire_value(raw_status);
    let message = parsed
        .get("version-message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|message| localize_wire_update_message(message, language))
        .unwrap_or_else(|| default_update_check_message(&status, language));
    let mut url = parsed
        .get("version-url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if url.is_none()
        && matches!(
            status,
            LegacyUpdateCheckStatus::Failed | LegacyUpdateCheckStatus::Unknown(_)
        )
        && user_initiated
    {
        url = Some(SYNCPLAY_DOWNLOAD_URL.to_owned());
    }
    let public_servers = parsed
        .get("public-servers")
        .and_then(Value::as_str)
        .map(parse_public_server_response)
        .transpose()?;

    Ok(LegacyUpdateCheckResult {
        status,
        message,
        url,
        public_servers,
        checked_at_utc: String::new(),
        user_initiated,
    })
}

fn http_client() -> Result<Client, reqwest::Error> {
    ensure_rustls_crypto_provider();
    Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("syncplay-rs-gui/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

fn env_response_override(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn ensure_rustls_crypto_provider() {
    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

fn sanitize_wordpress_public_server_response(body: &str) -> String {
    body.replace("<p>", "")
        .replace("</p>", "")
        .replace("<br />", "")
        .replace("&#8220;", "'")
        .replace("&#8221;", "'")
        .replace(":&#8217;", "'")
        .replace("&#8217;", "'")
        .replace("&#8242;", "'")
        .replace(['\n', '\r'], "")
}

fn sanitize_wordpress_update_check_response(body: &str) -> String {
    body.replace("<p>", "")
        .replace("</p>", "")
        .replace("<br />", "")
        .replace("&#8220;", "\"")
        .replace("&#8221;", "\"")
        .replace(['\n', '\r'], "")
}

fn legacy_update_check_platform_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

fn normalized_language(language: Option<&str>) -> &'static str {
    language
        .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        .unwrap_or("en")
}

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
    match normalized_language(language) {
        "de" => de,
        "es" => es,
        "eo" => eo,
        "fi" => fi,
        "fr" => fr,
        "it" => it,
        "pt_PT" | "pt_BR" => pt,
        "tr" => tr,
        "ru" => ru,
        "zh_CN" => zh_cn,
        "ko" => ko,
        _ => en,
    }
}

fn public_server_list_failed_message(language: Option<&str>) -> &'static str {
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

fn default_update_check_message(
    status: &LegacyUpdateCheckStatus,
    language: Option<&str>,
) -> String {
    match status {
        LegacyUpdateCheckStatus::UpToDate => localized_literal(
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
        .to_owned(),
        LegacyUpdateCheckStatus::UpdateAvailable => localized_literal(
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
        .to_owned(),
        LegacyUpdateCheckStatus::Failed | LegacyUpdateCheckStatus::Unknown(_) => {
            update_check_failed_notification_message(language)
        }
    }
}

fn update_check_failed_notification_message(language: Option<&str>) -> String {
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
    .replace("{}", LEGACY_SYNCPLAY_VERSION)
}

fn localize_wire_update_message(message: &str, language: Option<&str>) -> String {
    let trimmed = message.trim();
    match trimmed {
        "Syncplay is up to date" | "Syncplay is up to date." => {
            default_update_check_message(&LegacyUpdateCheckStatus::UpToDate, language)
        }
        "A new version of Syncplay is available. Do you want to visit the release page?" => {
            default_update_check_message(&LegacyUpdateCheckStatus::UpdateAvailable, language)
        }
        _ => trimmed.to_owned(),
    }
}

fn legacy_utc_timestamp_string_legacy_compatible(now: std::time::SystemTime) -> String {
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let total_seconds = duration.as_secs() as i64;
    let days_since_epoch = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days_since_unix_epoch_legacy_compatible(days_since_epoch);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let millis = duration.subsec_millis();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
}

fn parse_legacy_utc_timestamp_legacy_compatible(value: &str) -> Option<std::time::SystemTime> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 23
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
    {
        return None;
    }

    let year = value[0..4].parse::<i64>().ok()?;
    let month = value[5..7].parse::<u32>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let hour = value[11..13].parse::<u64>().ok()?;
    let minute = value[14..16].parse::<u64>().ok()?;
    let second = value[17..19].parse::<u64>().ok()?;
    let millis = value[20..23].parse::<u64>().ok()?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || millis > 999
    {
        return None;
    }

    let days_since_epoch =
        days_since_unix_epoch_from_civil_legacy_compatible(year, month as i64, day as i64);
    if days_since_epoch < 0 {
        return None;
    }

    let total_seconds = days_since_epoch as u64 * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(
        std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(total_seconds)
            + std::time::Duration::from_millis(millis),
    )
}

fn civil_from_days_since_unix_epoch_legacy_compatible(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn days_since_unix_epoch_from_civil_legacy_compatible(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let yoe = adjusted_year - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::{
        LegacyUpdateCheckStatus, StoredClientSettingsMvp, default_update_check_message,
        fetch_public_servers_from_url, fetch_update_check_result_from_url,
        parse_public_server_response, parse_update_check_response,
        sanitize_wordpress_public_server_response, sanitize_wordpress_update_check_response,
        should_run_automatic_update_check,
    };

    fn spawn_single_request_server(body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("test HTTP server should bind to localhost");
        let address = listener
            .local_addr()
            .expect("test HTTP server should expose a local address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("test HTTP server should accept a request");
            let mut buffer = [0u8; 4096];
            let bytes_read = stream
                .read(&mut buffer)
                .expect("test HTTP server should read the request");
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
            let request_line = request
                .lines()
                .next()
                .expect("HTTP request should contain a request line")
                .to_owned();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("test HTTP server should write the response");
            request_line
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn wordpress_public_server_response_cleanup_matches_python_rules() {
        let cleaned = sanitize_wordpress_public_server_response(
            "<p>[[' Primary ', ' syncplay.pl:8999 '], ['&#8220;Quoted&#8221;', 'beta.example:9000']]</p>\r\n",
        );
        assert_eq!(
            cleaned,
            "[[' Primary ', ' syncplay.pl:8999 '], [''Quoted'', 'beta.example:9000']]"
        );
    }

    #[test]
    fn public_server_response_parser_accepts_legacy_python_list_format() {
        let parsed = parse_public_server_response(
            "<p>[[' Primary ', ' syncplay.pl:8999 '], ['Backup', 'backup.example:9000']]</p>",
        )
        .expect("legacy public-server list should parse");

        assert_eq!(
            parsed,
            vec![
                (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
                ("Backup".to_owned(), "backup.example:9000".to_owned()),
            ]
        );
    }

    #[test]
    fn public_server_response_parser_rejects_empty_results() {
        let error = parse_public_server_response("[]").expect_err("empty list should fail");
        assert!(error.contains("returned no servers"));
    }

    #[test]
    fn wordpress_update_check_response_cleanup_matches_python_rules() {
        let cleaned = sanitize_wordpress_update_check_response(
            "<p>{&#8220;version-status&#8221;: &#8220;uptodate&#8221;}</p>\r\n",
        );
        assert_eq!(cleaned, "{\"version-status\": \"uptodate\"}");
    }

    #[test]
    fn update_check_response_parser_accepts_legacy_json_and_public_servers() {
        let parsed = parse_update_check_response(
            r#"<p>{"version-status":"updateavailale","version-message":"New build available.","version-url":"https://syncplay.pl/download/","public-servers":"[['Primary','syncplay.pl:8999']]"}</p>"#,
            Some("en"),
            true,
        )
        .expect("legacy update response should parse");

        assert_eq!(parsed.status, LegacyUpdateCheckStatus::UpdateAvailable);
        assert_eq!(parsed.message, "New build available.");
        assert_eq!(parsed.url.as_deref(), Some("https://syncplay.pl/download/"));
        assert_eq!(
            parsed.public_servers,
            Some(vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())])
        );
    }

    #[test]
    fn update_check_response_parser_falls_back_to_default_failure_message_for_unknown_status() {
        let parsed =
            parse_update_check_response(r#"{"version-status":"mystery"}"#, Some("en"), true)
                .expect("unknown status should still parse");

        assert_eq!(
            parsed.status,
            LegacyUpdateCheckStatus::Unknown("mystery".to_owned())
        );
        assert_eq!(
            parsed.message,
            default_update_check_message(
                &LegacyUpdateCheckStatus::Unknown("mystery".to_owned()),
                Some("en"),
            )
        );
        assert_eq!(parsed.url.as_deref(), Some("https://syncplay.pl/download/"));
    }

    #[test]
    fn public_server_request_uses_selected_language_query_parameter() {
        let (url, request_handle) = spawn_single_request_server("[['Primary','syncplay.pl:8999']]");

        let parsed = fetch_public_servers_from_url(&url, Some("fr"))
            .expect("public-server request should parse the server response");

        assert_eq!(
            parsed,
            vec![("Primary".to_owned(), "syncplay.pl:8999".to_owned())]
        );
        let request_line = request_handle
            .join()
            .expect("request capture thread should complete");
        assert!(request_line.contains("language=fr"));
    }

    #[test]
    fn update_check_request_uses_selected_language_query_parameter_and_localizes_defaults() {
        let (url, request_handle) = spawn_single_request_server(r#"{"version-status":"uptodate"}"#);

        let parsed = fetch_update_check_result_from_url(&url, Some("fr"), true)
            .expect("update-check request should parse the server response");

        assert_eq!(parsed.status, LegacyUpdateCheckStatus::UpToDate);
        assert_eq!(parsed.message, "Syncplay est a jour");
        let request_line = request_handle
            .join()
            .expect("request capture thread should complete");
        assert!(request_line.contains("language=fr"));
        assert!(request_line.contains("userInitiated=True"));
    }

    #[test]
    fn automatic_update_check_runs_when_timestamp_is_missing_or_stale() {
        let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let stale = super::legacy_utc_timestamp_string_legacy_compatible(
            now - Duration::from_secs(super::LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS + 1),
        );
        let fresh = super::legacy_utc_timestamp_string_legacy_compatible(
            now - Duration::from_secs(super::LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS - 1),
        );

        assert!(should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: None,
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
        assert!(should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: Some(stale),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
        assert!(!should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(true),
                last_checked_for_updates: Some(fresh),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
        assert!(!should_run_automatic_update_check(
            Some(&StoredClientSettingsMvp {
                check_for_updates_automatically: Some(false),
                last_checked_for_updates: Some(
                    super::legacy_utc_timestamp_string_legacy_compatible(SystemTime::now())
                ),
                ..StoredClientSettingsMvp::default()
            }),
            now,
        ));
    }
}
