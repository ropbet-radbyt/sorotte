pub mod auth;
pub mod cache;
pub mod discovery;
pub mod http;
pub mod library;
pub mod resolver;
pub mod timeline;

use std::{
    collections::BTreeMap,
    fmt, fs,
    path::Path,
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sorotte_player_api::LocalFileUpdate;
use sorotte_secret::SecretValue;

const DEFAULT_PLEX_TV_BASE_URL: &str = "https://plex.tv";
const DEFAULT_PLEX_AUTH_APP_URL: &str = "https://app.plex.tv/auth";
const DEFAULT_CLIENT_PRODUCT: &str = "Sorotte";
const DEFAULT_CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_TIMELINE_INTERVAL: Duration = Duration::from_secs(10);
const MATCH_RETRY_INTERVAL: Duration = Duration::from_secs(10);
const SEEK_REPORT_THRESHOLD_MILLIS: i64 = 15_000;
static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

pub type PlexResult<T> = Result<T, PlexError>;

#[derive(Debug)]
pub enum PlexError {
    Http(String),
    Json(serde_json::Error),
    Io(std::io::Error),
    InvalidResponse(String),
    MissingServer,
    MissingToken,
}

impl fmt::Display for PlexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(message) => write!(
                f,
                "failed Plex HTTP request: {}",
                redact_plex_token(message)
            ),
            Self::Json(error) => write!(f, "failed to parse Plex response: {error}"),
            Self::Io(error) => write!(f, "failed to read or write Plex cache: {error}"),
            Self::InvalidResponse(message) => write!(
                f,
                "Plex returned an invalid response: {}",
                redact_plex_token(message)
            ),
            Self::MissingServer => write!(f, "Plex server is not configured"),
            Self::MissingToken => write!(f, "Plex token is not configured"),
        }
    }
}

impl std::error::Error for PlexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Http(_) | Self::InvalidResponse(_) | Self::MissingServer | Self::MissingToken => {
                None
            }
        }
    }
}

impl From<reqwest::Error> for PlexError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(redact_plex_token(&value.to_string()))
    }
}

impl From<serde_json::Error> for PlexError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<std::io::Error> for PlexError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexAuthSession {
    pub pin_id: u64,
    pub code: String,
    pub auth_url: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexAuthPollResult {
    pub auth_token: Option<SecretValue>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlexClientConfig {
    pub enabled: bool,
    pub streaming_enabled: bool,
    pub user_token: Option<SecretValue>,
    pub selected_server_id: Option<String>,
    pub selected_server_url: Option<String>,
    pub selected_server_token: Option<SecretValue>,
}

impl PlexClientConfig {
    pub fn selected_server_token_or_user_token(&self) -> Option<&str> {
        self.selected_server_token
            .as_ref()
            .or(self.user_token.as_ref())
            .map(SecretValue::expose_secret)
            .filter(|token| !token.trim().is_empty())
    }

    pub fn has_selected_server(&self) -> bool {
        self.selected_server_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            && self.selected_server_token_or_user_token().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlexServerConnectionKind {
    Local,
    #[default]
    Remote,
    Relay,
}

impl PlexServerConnectionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Relay => "relay",
        }
    }

    pub fn is_local(self) -> bool {
        self == Self::Local
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexServerConnection {
    pub name: String,
    pub machine_identifier: String,
    pub uri: String,
    pub access_token: SecretValue,
    pub owned: bool,
    pub has_local_connection: bool,
    pub connection_kind: PlexServerConnectionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlexWatchEvent {
    pub file: LocalFileUpdate,
    pub position_seconds: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub paused: Option<bool>,
    pub changed_at: SystemTime,
}

impl PlexWatchEvent {
    pub fn new(file: LocalFileUpdate) -> Self {
        let duration_seconds = file.duration_seconds;
        Self {
            file,
            position_seconds: None,
            duration_seconds,
            paused: None,
            changed_at: SystemTime::now(),
        }
    }

    pub fn with_position_seconds(mut self, position_seconds: f64) -> Self {
        self.position_seconds = Some(position_seconds);
        self
    }

    pub fn with_duration_seconds(mut self, duration_seconds: f64) -> Self {
        self.duration_seconds = Some(duration_seconds);
        self
    }

    pub fn with_paused(mut self, paused: bool) -> Self {
        self.paused = Some(paused);
        self
    }

    pub fn with_changed_at(mut self, changed_at: SystemTime) -> Self {
        self.changed_at = changed_at;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlexMediaType {
    Movie,
    Episode,
    Other,
}

impl PlexMediaType {
    fn from_plex_type(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Self::Movie,
            "episode" => Self::Episode,
            _ => Self::Other,
        }
    }

    pub fn is_video_watch_type(self) -> bool {
        matches!(self, Self::Movie | Self::Episode)
    }

    fn as_playlist_uri_type(self) -> Option<&'static str> {
        match self {
            Self::Movie => Some("movie"),
            Self::Episode => Some("episode"),
            Self::Other => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexPlaylistUri {
    pub machine_identifier: String,
    pub rating_key: String,
    pub title: Option<String>,
    pub file_name: Option<String>,
    pub duration_millis: Option<u64>,
    pub size_bytes: Option<u64>,
    pub media_type: Option<PlexMediaType>,
}

impl fmt::Display for PlexPlaylistUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_plex_playlist_uri(self))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlexMatchedItem {
    pub rating_key: String,
    pub title: String,
    pub media_type: PlexMediaType,
    pub duration_millis: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexMediaSearchResult {
    pub rating_key: String,
    pub title: String,
    pub parent_title: Option<String>,
    pub grandparent_title: Option<String>,
    pub media_type: PlexMediaType,
    pub duration_millis: Option<u64>,
    pub file_paths: Vec<String>,
}

impl PlexMediaSearchResult {
    pub fn into_matched_item(self) -> PlexMatchedItem {
        PlexMatchedItem {
            rating_key: self.rating_key,
            title: self.title,
            media_type: self.media_type,
            duration_millis: self.duration_millis,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexPlayablePart {
    pub id: String,
    pub key: String,
    pub file_name: Option<String>,
    pub duration_millis: Option<u64>,
    pub size_bytes: Option<u64>,
    pub container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexMediaMetadata {
    pub rating_key: String,
    pub title: String,
    pub media_type: PlexMediaType,
    pub duration_millis: Option<u64>,
    pub parts: Vec<PlexPlayablePart>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretPlexPlaybackUrl(String);

impl SecretPlexPlaybackUrl {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for SecretPlexPlaybackUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretPlexPlaybackUrl")
            .field(&redact_plex_token(&self.0))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlexStreamTarget {
    pub playlist_uri: PlexPlaylistUri,
    pub matched_item: PlexMatchedItem,
    pub logical_file: LocalFileUpdate,
    pub playback_url: SecretPlexPlaybackUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlexTimelineState {
    Playing,
    Paused,
    Stopped,
}

impl PlexTimelineState {
    pub fn as_plex_value(self) -> &'static str {
        match self {
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexTimelineReport {
    pub rating_key: String,
    pub state: PlexTimelineState,
    pub time_millis: u64,
    pub duration_millis: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlexSyncState {
    Disconnected,
    Authenticating,
    Ready,
    Syncing,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexSyncStatus {
    pub state: PlexSyncState,
    pub current_item: Option<PlexMatchedItem>,
    pub last_report_at: Option<SystemTime>,
    pub last_error: Option<String>,
}

impl Default for PlexSyncStatus {
    fn default() -> Self {
        Self {
            state: PlexSyncState::Disconnected,
            current_item: None,
            last_report_at: None,
            last_error: None,
        }
    }
}

impl PlexSyncStatus {
    pub fn disconnected() -> Self {
        Self::default()
    }

    pub fn ready() -> Self {
        Self {
            state: PlexSyncState::Ready,
            ..Self::default()
        }
    }

    pub fn error(message: impl Into<String>, current_item: Option<PlexMatchedItem>) -> Self {
        Self {
            state: PlexSyncState::Error,
            current_item,
            last_report_at: None,
            last_error: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlexCachedMatch {
    pub rating_key: String,
    pub title: String,
    pub media_type: PlexMediaType,
    pub duration_millis: Option<u64>,
}

impl From<PlexMatchedItem> for PlexCachedMatch {
    fn from(value: PlexMatchedItem) -> Self {
        Self {
            rating_key: value.rating_key,
            title: value.title,
            media_type: value.media_type,
            duration_millis: value.duration_millis,
        }
    }
}

impl From<PlexCachedMatch> for PlexMatchedItem {
    fn from(value: PlexCachedMatch) -> Self {
        Self {
            rating_key: value.rating_key,
            title: value.title,
            media_type: value.media_type,
            duration_millis: value.duration_millis,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlexMatchCache {
    pub entries: BTreeMap<String, PlexCachedMatch>,
}

impl PlexMatchCache {
    pub fn load_from_path(path: &Path) -> PlexResult<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub fn save_to_path(&self, path: &Path) -> PlexResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn get_for_file(&self, file: &LocalFileUpdate) -> Option<PlexMatchedItem> {
        let key = cache_key_for_file(file)?;
        self.entries.get(&key).cloned().map(Into::into)
    }

    pub fn put_for_file(
        &mut self,
        file: &LocalFileUpdate,
        item: PlexMatchedItem,
    ) -> Option<String> {
        let key = cache_key_for_file(file)?;
        self.entries.insert(key.clone(), item.into());
        Some(key)
    }
}

#[derive(Debug, Clone)]
pub struct PlexHttpClient {
    client: Client,
    plex_tv_base_url: String,
    auth_app_url: String,
    client_identifier: String,
    product: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlexLibrarySection {
    key: String,
    library_type: String,
}

impl PlexHttpClient {
    pub fn new(client_identifier: impl Into<String>) -> PlexResult<Self> {
        Self::with_base_urls(
            DEFAULT_PLEX_TV_BASE_URL,
            DEFAULT_PLEX_AUTH_APP_URL,
            client_identifier,
            DEFAULT_CLIENT_PRODUCT,
        )
    }

    pub fn with_base_urls(
        plex_tv_base_url: impl Into<String>,
        auth_app_url: impl Into<String>,
        client_identifier: impl Into<String>,
        product: impl Into<String>,
    ) -> PlexResult<Self> {
        ensure_rustls_crypto_provider();
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(format!("sorotte-plex/{}", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            client,
            plex_tv_base_url: plex_tv_base_url.into().trim_end_matches('/').to_owned(),
            auth_app_url: auth_app_url.into(),
            client_identifier: client_identifier.into(),
            product: product.into(),
        })
    }

    pub fn start_auth(&self) -> PlexResult<PlexAuthSession> {
        let url = format!("{}/api/v2/pins", self.plex_tv_base_url);
        let response = self
            .client
            .post(url)
            .headers(self.plex_headers(None))
            .query(&[("strong", "true")])
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "PIN auth start returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        parse_auth_session_response(
            &json,
            &self.auth_app_url,
            &self.client_identifier,
            &self.product,
        )
    }

    pub fn poll_auth(&self, pin_id: u64) -> PlexResult<PlexAuthPollResult> {
        let url = format!("{}/api/v2/pins/{pin_id}", self.plex_tv_base_url);
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(None))
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "PIN auth poll returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        Ok(PlexAuthPollResult {
            auth_token: json
                .get("authToken")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(SecretValue::from),
            expires_at: json
                .get("expiresAt")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        })
    }

    pub fn discover_servers(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>> {
        if user_token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        let mut output = Vec::new();
        let mut first_error = None;
        let mut saw_success = false;
        for url in self.resources_urls() {
            match self.fetch_server_resources(user_token, &url) {
                Ok(servers) => {
                    saw_success = true;
                    merge_server_connections(&mut output, servers);
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if saw_success {
            Ok(output)
        } else {
            Err(first_error.unwrap_or_else(|| {
                PlexError::InvalidResponse(
                    "server discovery did not attempt any resources endpoint".to_owned(),
                )
            }))
        }
    }

    fn resources_urls(&self) -> Vec<String> {
        if self.plex_tv_base_url == DEFAULT_PLEX_TV_BASE_URL {
            vec![
                "https://clients.plex.tv/api/v2/resources".to_owned(),
                "https://plex.tv/api/v2/resources".to_owned(),
                "https://plex.tv/api/resources".to_owned(),
                "https://plex.tv/resources".to_owned(),
            ]
        } else {
            vec![
                format!("{}/api/v2/resources", self.plex_tv_base_url),
                format!("{}/api/resources", self.plex_tv_base_url),
                format!("{}/resources", self.plex_tv_base_url),
            ]
        }
    }

    fn fetch_server_resources(
        &self,
        user_token: &str,
        url: &str,
    ) -> PlexResult<Vec<PlexServerConnection>> {
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(user_token)))
            .query(&[
                ("includeHttps", "1"),
                ("includeRelay", "1"),
                ("includeIPv6", "1"),
            ])
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "server discovery returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        Ok(parse_server_resources_response(&json))
    }

    pub fn verify_server_connection(&self, server: &PlexServerConnection) -> PlexResult<()> {
        if server.access_token.expose_secret().trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        let response = self
            .client
            .get(server.uri.trim_end_matches('/'))
            .headers(self.plex_headers(Some(server.access_token.expose_secret())))
            .send()?;
        if !response.status().is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "server verification returned HTTP {}",
                response.status()
            )));
        }
        Ok(())
    }

    pub fn search_media_by_file_name(
        &self,
        server_url: &str,
        token: &str,
        file_name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        if token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        if file_name.trim().is_empty() {
            return Ok(Vec::new());
        }

        let sections = self.fetch_library_sections(server_url, token)?;
        let mut output = Vec::new();
        for section in sections {
            for media_type in library_section_media_type_filters(&section.library_type) {
                let results = self.fetch_library_section_media_by_file_name(
                    server_url,
                    token,
                    &section.key,
                    media_type,
                    file_name,
                )?;
                merge_media_search_results(&mut output, results);
            }
        }
        Ok(output)
    }

    pub fn search_selected_server_media(
        &self,
        config: &PlexClientConfig,
        query: &str,
        limit: usize,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let (server_url, token) = configured_server_url_and_token(config)?;
        let sections = self.fetch_library_sections(&server_url, &token)?;
        let mut output = Vec::new();
        let query = query.trim();
        for section in sections {
            for media_type in library_section_media_type_filters(&section.library_type) {
                let remaining = limit.saturating_sub(output.len()).max(1);
                let results = if query.is_empty() {
                    self.fetch_recent_library_section_media(
                        &server_url,
                        &token,
                        &section.key,
                        media_type,
                        remaining,
                    )?
                } else {
                    self.fetch_library_section_media_by_query(
                        &server_url,
                        &token,
                        &section.key,
                        media_type,
                        query,
                        remaining,
                    )?
                };
                merge_media_search_results(&mut output, results);
                output.retain(|result| result.media_type.is_video_watch_type());
                if output.len() >= limit {
                    output.truncate(limit);
                    return Ok(output);
                }
            }
        }
        output.truncate(limit);
        Ok(output)
    }

    pub fn playlist_uri_for_selected_server_rating_key(
        &self,
        config: &PlexClientConfig,
        rating_key: &str,
    ) -> PlexResult<PlexPlaylistUri> {
        let (server_url, token) = configured_server_url_and_token(config)?;
        let machine_identifier =
            selected_server_machine_identifier_with_transport(config, self, &server_url, &token)?;
        let metadata = self.metadata_by_rating_key(&server_url, &token, rating_key)?;
        playlist_uri_for_metadata(&machine_identifier, &metadata, None)
    }

    pub fn server_machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String> {
        if token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        let response = self
            .client
            .get(server_url.trim_end_matches('/'))
            .headers(self.plex_headers(Some(token)))
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "server identity lookup returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        parse_server_machine_identifier_response(&json)
    }

    fn fetch_library_sections(
        &self,
        server_url: &str,
        token: &str,
    ) -> PlexResult<Vec<PlexLibrarySection>> {
        let url = format!("{}/library/sections", server_url.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(token)))
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "library sections lookup returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        Ok(parse_library_sections_response(&json))
    }

    fn fetch_library_section_media_by_file_name(
        &self,
        server_url: &str,
        token: &str,
        section_key: &str,
        media_type: &str,
        file_name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        let url = format!(
            "{}/library/sections/{}/all",
            server_url.trim_end_matches('/'),
            percent_encode_path_segment(section_key)
        );
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(token)))
            .query(&[
                ("type", media_type),
                ("includeGuids", "1"),
                ("file", file_name),
            ])
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "library file lookup returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        Ok(parse_search_response(&json))
    }

    fn fetch_library_section_media_by_query(
        &self,
        server_url: &str,
        token: &str,
        section_key: &str,
        media_type: &str,
        query: &str,
        limit: usize,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        let mut output = Vec::new();
        for query_filter in library_section_text_query_filters(media_type) {
            let results = self.fetch_library_section_media(
                server_url,
                token,
                section_key,
                &[
                    ("type".to_owned(), media_type.to_owned()),
                    ("includeGuids".to_owned(), "1".to_owned()),
                    (query_filter.to_owned(), query.to_owned()),
                    ("X-Plex-Container-Start".to_owned(), "0".to_owned()),
                    ("X-Plex-Container-Size".to_owned(), limit.max(1).to_string()),
                ],
                "library text lookup",
            )?;
            merge_media_search_results(
                &mut output,
                filter_media_search_results_by_query(results, query),
            );
            if output.len() >= limit {
                output.truncate(limit);
                return Ok(output);
            }
        }
        output.truncate(limit);
        Ok(output)
    }

    fn fetch_recent_library_section_media(
        &self,
        server_url: &str,
        token: &str,
        section_key: &str,
        media_type: &str,
        limit: usize,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        self.fetch_library_section_media(
            server_url,
            token,
            section_key,
            &[
                ("type".to_owned(), media_type.to_owned()),
                ("includeGuids".to_owned(), "1".to_owned()),
                ("sort".to_owned(), "addedAt:desc".to_owned()),
                ("X-Plex-Container-Start".to_owned(), "0".to_owned()),
                ("X-Plex-Container-Size".to_owned(), limit.max(1).to_string()),
            ],
            "library recent lookup",
        )
    }

    fn fetch_library_section_media(
        &self,
        server_url: &str,
        token: &str,
        section_key: &str,
        query: &[(String, String)],
        label: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        let url = format!(
            "{}/library/sections/{}/all",
            server_url.trim_end_matches('/'),
            percent_encode_path_segment(section_key)
        );
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(token)))
            .query(query)
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "{label} returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        Ok(parse_search_response(&json))
    }

    pub fn metadata_by_rating_key(
        &self,
        server_url: &str,
        token: &str,
        rating_key: &str,
    ) -> PlexResult<PlexMediaMetadata> {
        if token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        if rating_key.trim().is_empty() {
            return Err(PlexError::InvalidResponse(
                "metadata lookup requires a rating key".to_owned(),
            ));
        }
        let url = format!(
            "{}/library/metadata/{}",
            server_url.trim_end_matches('/'),
            percent_encode_path_segment(rating_key)
        );
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(token)))
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "metadata lookup returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        parse_metadata_response(&json, rating_key)
    }

    pub fn build_part_stream_url(
        &self,
        server_url: &str,
        token: &str,
        part: &PlexPlayablePart,
    ) -> PlexResult<SecretPlexPlaybackUrl> {
        if token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        if part.key.trim().is_empty() {
            return Err(PlexError::InvalidResponse(
                "metadata part did not include a stream key".to_owned(),
            ));
        }
        let base = server_url.trim_end_matches('/');
        let part_key = part.key.trim();
        let mut url = if part_key.starts_with("http://") || part_key.starts_with("https://") {
            part_key.to_owned()
        } else if part_key.starts_with('/') {
            format!("{base}{part_key}")
        } else {
            format!("{base}/{part_key}")
        };
        let delimiter = if url.contains('?') { '&' } else { '?' };
        url.push(delimiter);
        url.push_str("X-Plex-Token=");
        url.push_str(&percent_encode_query_value(token));
        Ok(SecretPlexPlaybackUrl::new(url))
    }

    fn plex_headers(&self, token: Option<&str>) -> reqwest::header::HeaderMap {
        use reqwest::header::{ACCEPT, HeaderMap, HeaderName, HeaderValue};

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        insert_header_value(
            &mut headers,
            HeaderName::from_static("x-plex-client-identifier"),
            &self.client_identifier,
        );
        insert_header_value(
            &mut headers,
            HeaderName::from_static("x-plex-product"),
            &self.product,
        );
        insert_header_value(
            &mut headers,
            HeaderName::from_static("x-plex-version"),
            DEFAULT_CLIENT_VERSION,
        );
        insert_header_value(
            &mut headers,
            HeaderName::from_static("x-plex-platform"),
            plex_client_platform(),
        );
        insert_header_value(
            &mut headers,
            HeaderName::from_static("x-plex-device"),
            plex_client_platform(),
        );
        insert_header_value(
            &mut headers,
            HeaderName::from_static("x-plex-device-name"),
            &self.product,
        );
        if let Some(token) = token {
            insert_header_value(&mut headers, HeaderName::from_static("x-plex-token"), token);
        }
        headers
    }

    fn append_plex_identity_query_params(&self, query: &mut Vec<(String, String)>) {
        push_query_param(query, "X-Plex-Client-Identifier", &self.client_identifier);
        push_query_param(query, "X-Plex-Product", &self.product);
        push_query_param(query, "X-Plex-Version", DEFAULT_CLIENT_VERSION);
        push_query_param(query, "X-Plex-Platform", plex_client_platform());
        push_query_param(query, "X-Plex-Device", plex_client_platform());
        push_query_param(query, "X-Plex-Device-Name", &self.product);
    }
}

impl PlexSyncTransport for PlexHttpClient {
    fn search_media(
        &self,
        server_url: &str,
        token: &str,
        query: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        if token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        let url = format!("{}/search", server_url.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(token)))
            .query(&[("query", query)])
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "media search returned HTTP {status}"
            )));
        }
        let json: Value = serde_json::from_str(&body)?;
        Ok(parse_search_response(&json))
    }

    fn search_media_by_file_name(
        &self,
        server_url: &str,
        token: &str,
        file_name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        PlexHttpClient::search_media_by_file_name(self, server_url, token, file_name)
    }

    fn report_timeline(
        &self,
        server_url: &str,
        token: &str,
        report: &PlexTimelineReport,
    ) -> PlexResult<()> {
        if token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        let url = format!("{}/:/timeline", server_url.trim_end_matches('/'));
        let mut query = vec![
            ("ratingKey".to_owned(), report.rating_key.clone()),
            ("state".to_owned(), report.state.as_plex_value().to_owned()),
        ];
        query.push(("time".to_owned(), report.time_millis.to_string()));
        query.push((
            "duration".to_owned(),
            report.duration_millis.unwrap_or(0).to_string(),
        ));
        self.append_plex_identity_query_params(&mut query);

        let response = self
            .client
            .get(url)
            .headers(self.plex_headers(Some(token)))
            .query(&query)
            .send()?;
        if !response.status().is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "timeline report returned HTTP {}",
                response.status()
            )));
        }
        Ok(())
    }
}

impl PlexMetadataTransport for PlexHttpClient {
    fn metadata_by_rating_key(
        &self,
        server_url: &str,
        token: &str,
        rating_key: &str,
    ) -> PlexResult<PlexMediaMetadata> {
        PlexHttpClient::metadata_by_rating_key(self, server_url, token, rating_key)
    }

    fn build_part_stream_url(
        &self,
        server_url: &str,
        token: &str,
        part: &PlexPlayablePart,
    ) -> PlexResult<SecretPlexPlaybackUrl> {
        PlexHttpClient::build_part_stream_url(self, server_url, token, part)
    }

    fn server_machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String> {
        PlexHttpClient::server_machine_identifier(self, server_url, token)
    }
}

pub trait PlexServerDiscoveryTransport {
    fn discover_servers(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>>;
}

impl PlexServerDiscoveryTransport for PlexHttpClient {
    fn discover_servers(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>> {
        PlexHttpClient::discover_servers(self, user_token)
    }
}

pub trait PlexSyncTransport {
    fn search_media(
        &self,
        server_url: &str,
        token: &str,
        query: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>>;

    fn search_media_by_file_name(
        &self,
        server_url: &str,
        token: &str,
        file_name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>>;

    fn report_timeline(
        &self,
        server_url: &str,
        token: &str,
        report: &PlexTimelineReport,
    ) -> PlexResult<()>;
}

pub trait PlexMetadataTransport {
    fn metadata_by_rating_key(
        &self,
        server_url: &str,
        token: &str,
        rating_key: &str,
    ) -> PlexResult<PlexMediaMetadata>;

    fn build_part_stream_url(
        &self,
        server_url: &str,
        token: &str,
        part: &PlexPlayablePart,
    ) -> PlexResult<SecretPlexPlaybackUrl>;

    fn server_machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String>;
}

#[derive(Debug, Clone)]
pub struct PlexMediaResolver<T> {
    config: PlexClientConfig,
    transport: T,
    cache: PlexMatchCache,
    unmatched_keys: BTreeMap<String, SystemTime>,
}

impl<T> PlexMediaResolver<T>
where
    T: PlexSyncTransport + PlexMetadataTransport + PlexServerDiscoveryTransport,
{
    pub fn new(config: PlexClientConfig, transport: T, cache: PlexMatchCache) -> Self {
        Self {
            config,
            transport,
            cache,
            unmatched_keys: BTreeMap::new(),
        }
    }

    pub fn config(&self) -> &PlexClientConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: PlexClientConfig) {
        if self.config != config {
            self.cache = PlexMatchCache::default();
            self.unmatched_keys.clear();
            self.config = config;
        }
    }

    pub fn cache(&self) -> &PlexMatchCache {
        &self.cache
    }

    pub fn cache_mut(&mut self) -> &mut PlexMatchCache {
        &mut self.cache
    }

    pub fn into_parts(self) -> (PlexClientConfig, T, PlexMatchCache) {
        (self.config, self.transport, self.cache)
    }

    pub fn resolve_match_for_local_file(
        &mut self,
        file: &LocalFileUpdate,
        now: SystemTime,
    ) -> PlexResult<Option<PlexMatchedItem>> {
        let (server_url, token) = configured_server_url_and_token(&self.config)?;
        let Some(file_key) = server_scoped_cache_key_for_file(&self.config, file) else {
            return Ok(None);
        };
        resolve_media_match_for_file(
            &self.transport,
            &mut self.cache,
            &mut self.unmatched_keys,
            PlexMatchServerRef {
                url: &server_url,
                token: &token,
            },
            file,
            &file_key,
            now,
        )
    }

    pub fn resolve_match_for_playlist_uri(
        &self,
        uri: &PlexPlaylistUri,
    ) -> PlexResult<PlexMatchedItem> {
        let (server_url, token) = self.server_url_and_token_for_playlist_uri(uri)?;
        let metadata =
            self.transport
                .metadata_by_rating_key(&server_url, &token, &uri.rating_key)?;
        if !metadata.media_type.is_video_watch_type() {
            return Err(PlexError::InvalidResponse(format!(
                "Plex metadata {} is not playable video media",
                metadata.rating_key
            )));
        }
        Ok(PlexMatchedItem {
            rating_key: metadata.rating_key,
            title: metadata.title,
            media_type: metadata.media_type,
            duration_millis: metadata.duration_millis.or(uri.duration_millis),
        })
    }

    pub fn resolve_stream_target(
        &mut self,
        target: &str,
        now: SystemTime,
    ) -> PlexResult<Option<PlexStreamTarget>> {
        if !self.config.streaming_enabled {
            return Ok(None);
        }
        if is_plex_playlist_uri(target) {
            let playlist_uri = parse_plex_playlist_uri(target)?;
            let (server_url, token) = self.server_url_and_token_for_playlist_uri(&playlist_uri)?;
            return self.resolve_stream_target_for_playlist_uri(&server_url, &token, playlist_uri);
        }

        let (server_url, token) = configured_server_url_and_token(&self.config)?;
        let mut file =
            LocalFileUpdate::new(path_file_name(target).unwrap_or_else(|| target.into()));
        if target.trim().contains('/')
            || target.trim().contains('\\')
            || Path::new(target).is_absolute()
        {
            file = file.with_path(target.to_owned());
        }
        let Some(matched_item) = self.resolve_match_for_local_file(&file, now)? else {
            return Ok(None);
        };
        let metadata =
            self.transport
                .metadata_by_rating_key(&server_url, &token, &matched_item.rating_key)?;
        if !metadata.media_type.is_video_watch_type() {
            return Err(PlexError::InvalidResponse(format!(
                "Plex metadata {} is not playable video media",
                metadata.rating_key
            )));
        }
        let machine_identifier = selected_server_machine_identifier_with_transport(
            &self.config,
            &self.transport,
            &server_url,
            &token,
        )?;
        let playlist_uri = playlist_uri_for_metadata(
            &machine_identifier,
            &metadata,
            matched_item.duration_millis,
        )?;
        self.stream_target_from_metadata(&server_url, &token, playlist_uri, matched_item, metadata)
            .map(Some)
    }

    fn resolve_stream_target_for_playlist_uri(
        &self,
        server_url: &str,
        token: &str,
        playlist_uri: PlexPlaylistUri,
    ) -> PlexResult<Option<PlexStreamTarget>> {
        let metadata =
            self.transport
                .metadata_by_rating_key(server_url, token, &playlist_uri.rating_key)?;
        if !metadata.media_type.is_video_watch_type() {
            return Err(PlexError::InvalidResponse(format!(
                "Plex metadata {} is not playable video media",
                metadata.rating_key
            )));
        }
        let matched_item = PlexMatchedItem {
            rating_key: metadata.rating_key.clone(),
            title: metadata.title.clone(),
            media_type: metadata.media_type,
            duration_millis: metadata.duration_millis.or(playlist_uri.duration_millis),
        };
        self.stream_target_from_metadata(server_url, token, playlist_uri, matched_item, metadata)
            .map(Some)
    }

    fn stream_target_from_metadata(
        &self,
        server_url: &str,
        token: &str,
        mut playlist_uri: PlexPlaylistUri,
        matched_item: PlexMatchedItem,
        metadata: PlexMediaMetadata,
    ) -> PlexResult<PlexStreamTarget> {
        let part = choose_playable_part(
            &metadata,
            matched_item
                .duration_millis
                .or(playlist_uri.duration_millis)
                .or(metadata.duration_millis),
        )?;
        if playlist_uri.title.is_none() {
            playlist_uri.title = Some(metadata.title.clone());
        }
        if playlist_uri.file_name.is_none() {
            playlist_uri.file_name = part.file_name.clone();
        }
        if playlist_uri.duration_millis.is_none() {
            playlist_uri.duration_millis = part.duration_millis.or(metadata.duration_millis);
        }
        if playlist_uri.size_bytes.is_none() {
            playlist_uri.size_bytes = part.size_bytes;
        }
        if playlist_uri.media_type.is_none() {
            playlist_uri.media_type = Some(metadata.media_type);
        }
        let logical_name = playlist_uri
            .file_name
            .clone()
            .or_else(|| playlist_uri.title.clone())
            .unwrap_or_else(|| metadata.title.clone());
        let mut logical_file =
            LocalFileUpdate::new(logical_name).with_path(format_plex_playlist_uri(&playlist_uri));
        if let Some(duration_millis) = playlist_uri.duration_millis {
            logical_file = logical_file.with_duration_seconds(duration_millis as f64 / 1000.0);
        }
        if let Some(size_bytes) = playlist_uri.size_bytes {
            logical_file = logical_file.with_size_bytes(size_bytes);
        }
        let playback_url = self
            .transport
            .build_part_stream_url(server_url, token, &part)?;
        Ok(PlexStreamTarget {
            playlist_uri,
            matched_item,
            logical_file,
            playback_url,
        })
    }

    fn server_url_and_token_for_playlist_uri(
        &self,
        uri: &PlexPlaylistUri,
    ) -> PlexResult<(String, String)> {
        if selected_server_matches_machine_identifier(&self.config, &uri.machine_identifier) {
            return configured_server_url_and_token(&self.config);
        }

        let user_token = config_user_token(&self.config)?;
        let servers = self.transport.discover_servers(user_token)?;
        servers
            .into_iter()
            .find(|server| server.machine_identifier == uri.machine_identifier)
            .map(|server| (server.uri, server.access_token.into_exposed_secret()))
            .ok_or_else(|| {
                PlexError::InvalidResponse(format!(
                    "Plex playlist URI targets server '{}' but that server was not found in the receiver's accessible Plex servers",
                    uri.machine_identifier
                ))
            })
    }
}

#[derive(Debug, Clone)]
pub struct PlexSyncEngine<T> {
    config: PlexClientConfig,
    transport: T,
    cache: PlexMatchCache,
    status: PlexSyncStatus,
    current_file_key: Option<String>,
    last_report_signature: Option<ReportSignature>,
    unmatched_keys: BTreeMap<String, SystemTime>,
    timeline_interval: Duration,
}

impl<T> PlexSyncEngine<T>
where
    T: PlexSyncTransport,
{
    pub fn new(config: PlexClientConfig, transport: T, cache: PlexMatchCache) -> Self {
        let status = if config.enabled {
            PlexSyncStatus::ready()
        } else {
            PlexSyncStatus::disconnected()
        };
        Self {
            config,
            transport,
            cache,
            status,
            current_file_key: None,
            last_report_signature: None,
            unmatched_keys: BTreeMap::new(),
            timeline_interval: DEFAULT_TIMELINE_INTERVAL,
        }
    }

    pub fn config(&self) -> &PlexClientConfig {
        &self.config
    }

    pub fn set_config(&mut self, config: PlexClientConfig) {
        if self.config != config {
            self.current_file_key = None;
            self.last_report_signature = None;
            self.unmatched_keys.clear();
            self.status = if config.enabled {
                PlexSyncStatus::ready()
            } else {
                PlexSyncStatus::disconnected()
            };
            self.config = config;
        }
    }

    pub fn cache(&self) -> &PlexMatchCache {
        &self.cache
    }

    pub fn cache_mut(&mut self) -> &mut PlexMatchCache {
        &mut self.cache
    }

    pub fn status(&self) -> PlexSyncStatus {
        self.status.clone()
    }

    pub fn set_timeline_interval(&mut self, interval: Duration) {
        self.timeline_interval = interval;
    }

    pub fn tick(&mut self, event: Option<PlexWatchEvent>, now: SystemTime) -> PlexSyncStatus {
        let result = self.try_tick(event, now);
        if let Err(error) = result {
            self.status =
                PlexSyncStatus::error(error.to_string(), self.status.current_item.clone());
        }
        self.status.clone()
    }

    fn try_tick(&mut self, event: Option<PlexWatchEvent>, now: SystemTime) -> PlexResult<()> {
        if !self.config.enabled {
            self.status = PlexSyncStatus::disconnected();
            return Ok(());
        }
        let Some(server_url) = self.config.selected_server_url.clone() else {
            self.status = PlexSyncStatus::ready();
            return Ok(());
        };
        let Some(token) = self
            .config
            .selected_server_token_or_user_token()
            .map(ToOwned::to_owned)
        else {
            self.status = PlexSyncStatus::ready();
            return Ok(());
        };
        let Some(event) = event else {
            self.report_stop_if_needed(&server_url, &token, now)?;
            self.status.state = PlexSyncState::Ready;
            return Ok(());
        };
        let Some(file_key) = server_scoped_cache_key_for_file(&self.config, &event.file) else {
            self.status = PlexSyncStatus::ready();
            return Ok(());
        };

        if self.current_file_key.as_deref() != Some(file_key.as_str()) {
            self.report_stop_if_needed(&server_url, &token, now)?;
            self.current_file_key = Some(file_key.clone());
            self.last_report_signature = None;
            self.status.current_item = None;
        }

        let Some(item) = self.resolve_match(&server_url, &token, &event, &file_key, now)? else {
            self.status = PlexSyncStatus {
                state: PlexSyncState::Ready,
                current_item: None,
                last_report_at: self.status.last_report_at,
                last_error: Some(format!("No unambiguous Plex match for {}", event.file.name)),
            };
            return Ok(());
        };

        let report = timeline_report_for_event(&event, &item);
        let signature = ReportSignature::from_report(&report, now);
        if self.should_report(&signature) {
            self.transport
                .report_timeline(&server_url, &token, &report)?;
            self.status = PlexSyncStatus {
                state: PlexSyncState::Syncing,
                current_item: Some(item),
                last_report_at: Some(now),
                last_error: None,
            };
            self.last_report_signature = Some(signature);
        } else {
            self.status.state = PlexSyncState::Ready;
            self.status.current_item = Some(item);
            self.status.last_error = None;
        }
        Ok(())
    }

    fn resolve_match(
        &mut self,
        server_url: &str,
        token: &str,
        event: &PlexWatchEvent,
        file_key: &str,
        now: SystemTime,
    ) -> PlexResult<Option<PlexMatchedItem>> {
        let resolved = resolve_media_match_for_file(
            &self.transport,
            &mut self.cache,
            &mut self.unmatched_keys,
            PlexMatchServerRef {
                url: server_url,
                token,
            },
            &event.file,
            file_key,
            now,
        )?;
        if let Some(item) = resolved.as_ref() {
            self.status.current_item = Some(item.clone());
        }
        Ok(resolved)
    }

    fn should_report(&self, signature: &ReportSignature) -> bool {
        let Some(previous) = self.last_report_signature.as_ref() else {
            return true;
        };
        if previous.rating_key != signature.rating_key || previous.state != signature.state {
            return true;
        }
        if (signature.position_millis as i64 - previous.position_millis as i64).abs()
            >= SEEK_REPORT_THRESHOLD_MILLIS
        {
            return true;
        }
        signature
            .reported_at
            .duration_since(previous.reported_at)
            .map(|elapsed| elapsed >= self.timeline_interval)
            .unwrap_or(true)
    }

    fn report_stop_if_needed(
        &mut self,
        server_url: &str,
        token: &str,
        now: SystemTime,
    ) -> PlexResult<()> {
        let Some(previous) = self.last_report_signature.as_ref() else {
            self.current_file_key = None;
            self.status.current_item = None;
            return Ok(());
        };
        if previous.state == PlexTimelineState::Stopped {
            self.current_file_key = None;
            self.status.current_item = None;
            return Ok(());
        }
        let report = PlexTimelineReport {
            rating_key: previous.rating_key.clone(),
            state: PlexTimelineState::Stopped,
            time_millis: previous.position_millis,
            duration_millis: previous.duration_millis,
        };
        self.transport.report_timeline(server_url, token, &report)?;
        self.last_report_signature = Some(ReportSignature::from_report(&report, now));
        self.status = PlexSyncStatus {
            state: PlexSyncState::Syncing,
            current_item: None,
            last_report_at: Some(now),
            last_error: None,
        };
        self.current_file_key = None;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportSignature {
    rating_key: String,
    state: PlexTimelineState,
    position_millis: u64,
    duration_millis: Option<u64>,
    reported_at: SystemTime,
}

impl ReportSignature {
    fn from_report(report: &PlexTimelineReport, reported_at: SystemTime) -> Self {
        Self {
            rating_key: report.rating_key.clone(),
            state: report.state,
            position_millis: report.time_millis,
            duration_millis: report.duration_millis,
            reported_at,
        }
    }
}

fn configured_server_url_and_token(config: &PlexClientConfig) -> PlexResult<(String, String)> {
    let server_url = config
        .selected_server_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(PlexError::MissingServer)?
        .to_owned();
    let token = config
        .selected_server_token_or_user_token()
        .ok_or(PlexError::MissingToken)?
        .to_owned();
    Ok((server_url, token))
}

fn configured_selected_server_machine_identifier(config: &PlexClientConfig) -> Option<&str> {
    config
        .selected_server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn selected_server_machine_identifier_with_transport<T>(
    config: &PlexClientConfig,
    transport: &T,
    server_url: &str,
    token: &str,
) -> PlexResult<String>
where
    T: PlexMetadataTransport,
{
    configured_selected_server_machine_identifier(config)
        .map(ToOwned::to_owned)
        .map(Ok)
        .unwrap_or_else(|| transport.server_machine_identifier(server_url, token))
}

fn selected_server_matches_machine_identifier(
    config: &PlexClientConfig,
    machine_identifier: &str,
) -> bool {
    config
        .selected_server_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|selected| selected == machine_identifier)
}

fn config_user_token(config: &PlexClientConfig) -> PlexResult<&str> {
    config
        .user_token
        .as_ref()
        .map(SecretValue::expose_secret)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(PlexError::MissingToken)
}

struct PlexMatchServerRef<'a> {
    url: &'a str,
    token: &'a str,
}

fn resolve_media_match_for_file<T>(
    transport: &T,
    cache: &mut PlexMatchCache,
    unmatched_keys: &mut BTreeMap<String, SystemTime>,
    server: PlexMatchServerRef<'_>,
    file: &LocalFileUpdate,
    file_key: &str,
    now: SystemTime,
) -> PlexResult<Option<PlexMatchedItem>>
where
    T: PlexSyncTransport,
{
    if let Some(cached) = cache.entries.get(file_key).cloned() {
        return Ok(Some(cached.into()));
    }
    if unmatched_keys
        .get(file_key)
        .and_then(|last_attempt| now.duration_since(*last_attempt).ok())
        .is_some_and(|elapsed| elapsed < MATCH_RETRY_INTERVAL)
    {
        return Ok(None);
    }
    if let Some(file_name) = media_file_name_for_file(file)
        && let Ok(results) =
            transport.search_media_by_file_name(server.url, server.token, &file_name)
        && let Some(item) = choose_file_path_media_match(file, &results)
    {
        cache
            .entries
            .insert(file_key.to_owned(), PlexCachedMatch::from(item.clone()));
        unmatched_keys.remove(file_key);
        return Ok(Some(item));
    }
    let query = media_search_query_for_file(file);
    if query.is_empty() {
        unmatched_keys.insert(file_key.to_owned(), now);
        return Ok(None);
    }
    let results = transport.search_media(server.url, server.token, &query)?;
    let matched = choose_best_media_match(file, &results);
    match matched {
        Some(item) => {
            cache
                .entries
                .insert(file_key.to_owned(), PlexCachedMatch::from(item.clone()));
            unmatched_keys.remove(file_key);
            Ok(Some(item))
        }
        None => {
            unmatched_keys.insert(file_key.to_owned(), now);
            Ok(None)
        }
    }
}

pub fn is_plex_playlist_uri(value: &str) -> bool {
    value
        .trim_start()
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("plex://"))
}

pub fn parse_plex_playlist_uri(value: &str) -> PlexResult<PlexPlaylistUri> {
    let value = value.trim();
    if !is_plex_playlist_uri(value) {
        return Err(PlexError::InvalidResponse(
            "Plex playlist URI must use the plex:// scheme".to_owned(),
        ));
    }
    if redact_plex_token(value) != value {
        return Err(PlexError::InvalidResponse(
            "Plex playlist URI must not include a token".to_owned(),
        ));
    }

    let rest = &value[7..];
    let (authority_and_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (machine_identifier, path) = authority_and_path
        .split_once('/')
        .unwrap_or((authority_and_path, ""));
    let machine_identifier = percent_decode_lossy(machine_identifier).trim().to_owned();
    if machine_identifier.is_empty() {
        return Err(PlexError::InvalidResponse(
            "Plex playlist URI is missing a machine identifier".to_owned(),
        ));
    }
    let path = path.trim_matches('/');
    if path.eq_ignore_ascii_case("metadata") {
        return Err(PlexError::InvalidResponse(
            "Plex playlist URI is missing a rating key".to_owned(),
        ));
    }
    let Some(rating_key) = path.strip_prefix("metadata/") else {
        return Err(PlexError::InvalidResponse(
            "Plex playlist URI must contain /metadata/{ratingKey}".to_owned(),
        ));
    };
    let rating_key = percent_decode_lossy(rating_key.trim_matches('/'))
        .trim()
        .to_owned();
    if rating_key.is_empty() {
        return Err(PlexError::InvalidResponse(
            "Plex playlist URI is missing a rating key".to_owned(),
        ));
    }

    let mut parsed = PlexPlaylistUri {
        machine_identifier,
        rating_key,
        title: None,
        file_name: None,
        duration_millis: None,
        size_bytes: None,
        media_type: None,
    };
    for (key, raw_value) in parse_query_pairs_lossy(query) {
        let key = key.trim().to_ascii_lowercase();
        if key.contains("token") {
            return Err(PlexError::InvalidResponse(
                "Plex playlist URI must not include a token".to_owned(),
            ));
        }
        match key.as_str() {
            "title" => {
                parsed.title = non_empty_string(raw_value);
            }
            "file" => {
                parsed.file_name = non_empty_string(
                    raw_value.and_then(|value| path_file_name(&value).or(Some(value))),
                );
            }
            "duration" => {
                parsed.duration_millis = raw_value
                    .as_deref()
                    .and_then(|value| value.trim().parse::<u64>().ok());
            }
            "size" => {
                parsed.size_bytes = raw_value
                    .as_deref()
                    .and_then(|value| value.trim().parse::<u64>().ok());
            }
            "type" => {
                parsed.media_type = raw_value
                    .as_deref()
                    .and_then(plex_playlist_media_type_from_value);
            }
            _ => {}
        }
    }
    Ok(parsed)
}

pub fn format_plex_playlist_uri(value: &PlexPlaylistUri) -> String {
    let mut output = format!(
        "plex://{}/metadata/{}",
        percent_encode_path_segment(&value.machine_identifier),
        percent_encode_path_segment(&value.rating_key)
    );
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(title) = value
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        query.push(("title", title.to_owned()));
    }
    if let Some(file_name) = value
        .file_name
        .as_deref()
        .and_then(path_file_name)
        .filter(|value| !value.trim().is_empty())
    {
        query.push(("file", file_name));
    } else if let Some(file_name) = value
        .file_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        query.push(("file", file_name.to_owned()));
    }
    let duration = value.duration_millis.map(|value| value.to_string());
    if let Some(duration) = duration.as_deref() {
        query.push(("duration", duration.to_owned()));
    }
    let size = value.size_bytes.map(|value| value.to_string());
    if let Some(size) = size.as_deref() {
        query.push(("size", size.to_owned()));
    }
    if let Some(media_type) = value
        .media_type
        .and_then(PlexMediaType::as_playlist_uri_type)
    {
        query.push(("type", media_type.to_owned()));
    }
    if !query.is_empty() {
        output.push('?');
        output.push_str(
            &query
                .into_iter()
                .map(|(key, item)| format!("{key}={}", percent_encode_query_value(&item)))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    output
}

pub fn playlist_uri_for_metadata(
    machine_identifier: &str,
    metadata: &PlexMediaMetadata,
    duration_hint_millis: Option<u64>,
) -> PlexResult<PlexPlaylistUri> {
    let preferred_part = choose_playable_part(metadata, duration_hint_millis)?;
    Ok(PlexPlaylistUri {
        machine_identifier: machine_identifier.to_owned(),
        rating_key: metadata.rating_key.clone(),
        title: Some(metadata.title.clone()),
        file_name: preferred_part.file_name.clone(),
        duration_millis: preferred_part.duration_millis.or(metadata.duration_millis),
        size_bytes: preferred_part.size_bytes,
        media_type: Some(metadata.media_type),
    })
}

fn choose_playable_part(
    metadata: &PlexMediaMetadata,
    duration_hint_millis: Option<u64>,
) -> PlexResult<PlexPlayablePart> {
    if !metadata.media_type.is_video_watch_type() {
        return Err(PlexError::InvalidResponse(format!(
            "Plex metadata {} is not playable video media",
            metadata.rating_key
        )));
    }
    let parts = metadata
        .parts
        .iter()
        .filter(|part| !part.key.trim().is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(PlexError::InvalidResponse(format!(
            "Plex metadata {} did not include a playable part",
            metadata.rating_key
        )));
    }
    let scored = parts
        .into_iter()
        .map(|part| {
            let score = match (duration_hint_millis, part.duration_millis) {
                (Some(hint), Some(duration)) => duration.abs_diff(hint),
                (Some(_), None) => u64::MAX,
                (None, _) => 0,
            };
            (score, part)
        })
        .collect::<Vec<_>>();
    let Some((best_score, best_part)) = scored.iter().min_by_key(|(score, _)| *score) else {
        return Err(PlexError::InvalidResponse(
            "Plex metadata did not include a playable part".to_owned(),
        ));
    };
    let equal_best = scored
        .iter()
        .filter(|(score, _)| score == best_score)
        .collect::<Vec<_>>();
    if equal_best.len() > 1 {
        return Err(PlexError::InvalidResponse(format!(
            "Plex metadata {} contains ambiguous playable parts",
            metadata.rating_key
        )));
    }
    Ok((*best_part).clone())
}

fn parse_metadata_response(
    json: &Value,
    requested_rating_key: &str,
) -> PlexResult<PlexMediaMetadata> {
    let metadata_items = collect_metadata_items(json);
    let selected = metadata_items
        .iter()
        .copied()
        .find(|item| {
            json_string(item, &["ratingKey", "key"])
                .as_deref()
                .is_some_and(|value| value == requested_rating_key)
        })
        .or_else(|| metadata_items.first().copied())
        .ok_or_else(|| {
            PlexError::InvalidResponse(format!(
                "metadata lookup for {requested_rating_key} returned no metadata"
            ))
        })?;
    parse_metadata_item(selected)
}

fn collect_metadata_items(json: &Value) -> Vec<&Value> {
    let mut output = media_container_items_any(json, &["Metadata", "metadata"]);
    if output.is_empty() {
        collect_metadata_items_recursive(json, &mut output);
    }
    output
}

fn collect_metadata_items_recursive<'a>(value: &'a Value, output: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if map
                .get("ratingKey")
                .or_else(|| map.get("key"))
                .and_then(value_as_string)
                .is_some()
                && map.get("type").and_then(Value::as_str).is_some()
            {
                output.push(value);
            }
            for child in map.values() {
                collect_metadata_items_recursive(child, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_metadata_items_recursive(item, output);
            }
        }
        _ => {}
    }
}

fn parse_metadata_item(item: &Value) -> PlexResult<PlexMediaMetadata> {
    let rating_key = json_string(item, &["ratingKey", "key"])
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            PlexError::InvalidResponse("metadata did not include ratingKey".to_owned())
        })?;
    let title = json_string(item, &["title", "grandparentTitle"])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());
    let media_type = item
        .get("type")
        .and_then(Value::as_str)
        .map(PlexMediaType::from_plex_type)
        .unwrap_or(PlexMediaType::Other);
    let duration_millis = item.get("duration").and_then(value_as_u64);
    let mut parts = Vec::new();
    collect_playable_parts(item, &mut parts);
    parts.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.id.cmp(&right.id))
    });
    parts.dedup_by(|left, right| left.key == right.key && left.id == right.id);
    Ok(PlexMediaMetadata {
        rating_key,
        title,
        media_type,
        duration_millis,
        parts,
    })
}

fn collect_playable_parts(value: &Value, output: &mut Vec<PlexPlayablePart>) {
    match value {
        Value::Object(map) => {
            if let Some(key) = map
                .get("key")
                .and_then(value_as_string)
                .filter(|value| value.starts_with("/library/parts/") || map.contains_key("file"))
            {
                let id = json_string(value, &["id"]).unwrap_or_else(|| key.clone());
                output.push(PlexPlayablePart {
                    id,
                    key,
                    file_name: map
                        .get("file")
                        .and_then(value_as_string)
                        .and_then(|value| path_file_name(&value)),
                    duration_millis: map.get("duration").and_then(value_as_u64),
                    size_bytes: map.get("size").and_then(value_as_u64),
                    container: map.get("container").and_then(value_as_string),
                });
            }
            for child in map.values() {
                collect_playable_parts(child, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_playable_parts(item, output);
            }
        }
        _ => {}
    }
}

pub fn cache_key_for_file(file: &LocalFileUpdate) -> Option<String> {
    if let Some(path) = file.path.as_deref() {
        let normalized = normalize_path_key(path);
        if !normalized.is_empty() {
            return Some(format!("path:{normalized}"));
        }
    }
    let name = normalized_title_stem(&file.name);
    if name.is_empty() {
        return None;
    }
    let duration = file
        .duration_seconds
        .and_then(seconds_to_millis)
        .map(|millis| millis.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    Some(format!("name:{name}:duration:{duration}"))
}

pub fn server_scoped_cache_key_for_file(
    config: &PlexClientConfig,
    file: &LocalFileUpdate,
) -> Option<String> {
    cache_key_for_file(file).map(|file_key| {
        format!(
            "server:{}:{file_key}",
            server_cache_scope_for_config(config)
        )
    })
}

fn server_cache_scope_for_config(config: &PlexClientConfig) -> String {
    config
        .selected_server_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("id:{}", normalized_cache_scope_value(value)))
        .or_else(|| {
            config
                .selected_server_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("url:{}", normalize_path_key(value)))
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn media_search_query_for_file(file: &LocalFileUpdate) -> String {
    normalized_title_stem(&file.name)
}

pub fn choose_best_media_match(
    file: &LocalFileUpdate,
    results: &[PlexMediaSearchResult],
) -> Option<PlexMatchedItem> {
    let query = media_search_query_for_file(file);
    let file_duration_millis = file.duration_seconds.and_then(seconds_to_millis);
    let mut scored = results
        .iter()
        .filter(|result| result.media_type.is_video_watch_type())
        .map(|result| {
            let title = normalized_title_stem(&result.title);
            let mut score = 30_i64;
            if title == query {
                score += 50;
            } else if !query.is_empty() && title.contains(&query) {
                score += 30;
            } else if !title.is_empty() && query.contains(&title) {
                score += 20;
            }
            if let (Some(file_duration), Some(result_duration)) =
                (file_duration_millis, result.duration_millis)
            {
                let delta = result_duration.abs_diff(file_duration);
                if delta <= 30_000 {
                    score += 35;
                } else if delta <= 90_000 {
                    score += 20;
                } else if delta > 10 * 60_000 {
                    score -= 30;
                }
            }
            (score, result)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.rating_key.cmp(&right.rating_key))
    });
    let (best_score, best) = scored.first()?;
    if *best_score < 50 {
        return None;
    }
    if let Some((second_score, _)) = scored.get(1)
        && best_score - second_score < 10
    {
        return None;
    }
    Some((*best).clone().into_matched_item())
}

pub fn choose_file_path_media_match(
    file: &LocalFileUpdate,
    results: &[PlexMediaSearchResult],
) -> Option<PlexMatchedItem> {
    let mut path_matches = results
        .iter()
        .filter(|result| {
            result.media_type.is_video_watch_type()
                && result
                    .file_paths
                    .iter()
                    .any(|plex_path| local_file_path_matches_plex_path(file, plex_path))
        })
        .collect::<Vec<_>>();
    path_matches.sort_by(|left, right| left.rating_key.cmp(&right.rating_key));
    path_matches.dedup_by(|left, right| left.rating_key == right.rating_key);
    if path_matches.len() == 1 {
        return Some(path_matches[0].clone().into_matched_item());
    }

    let local_file_name = media_file_name_for_file(file).map(|name| normalize_file_name(&name))?;
    let mut basename_matches = results
        .iter()
        .filter(|result| {
            result.media_type.is_video_watch_type()
                && result.file_paths.iter().any(|plex_path| {
                    path_file_name(plex_path)
                        .as_deref()
                        .map(normalize_file_name)
                        .is_some_and(|name| name == local_file_name)
                })
        })
        .collect::<Vec<_>>();
    basename_matches.sort_by(|left, right| left.rating_key.cmp(&right.rating_key));
    basename_matches.dedup_by(|left, right| left.rating_key == right.rating_key);
    if basename_matches.len() == 1 {
        return Some(basename_matches[0].clone().into_matched_item());
    }
    None
}

pub fn timeline_report_for_event(
    event: &PlexWatchEvent,
    item: &PlexMatchedItem,
) -> PlexTimelineReport {
    PlexTimelineReport {
        rating_key: item.rating_key.clone(),
        state: if event.paused.unwrap_or(false) {
            PlexTimelineState::Paused
        } else {
            PlexTimelineState::Playing
        },
        time_millis: event
            .position_seconds
            .and_then(seconds_to_millis)
            .unwrap_or(0),
        duration_millis: event
            .duration_seconds
            .or(event.file.duration_seconds)
            .and_then(seconds_to_millis)
            .or(item.duration_millis),
    }
}

fn parse_auth_session_response(
    json: &Value,
    auth_app_url: &str,
    client_identifier: &str,
    product: &str,
) -> PlexResult<PlexAuthSession> {
    let pin_id = json
        .get("id")
        .and_then(value_as_u64)
        .ok_or_else(|| PlexError::InvalidResponse("PIN response did not include id".to_owned()))?;
    let code = json
        .get("code")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| PlexError::InvalidResponse("PIN response did not include code".to_owned()))?
        .to_owned();
    let expires_at = json
        .get("expiresAt")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    Ok(PlexAuthSession {
        pin_id,
        auth_url: plex_auth_url(auth_app_url, client_identifier, &code, product),
        code,
        expires_at,
    })
}

fn parse_server_resources_response(json: &Value) -> Vec<PlexServerConnection> {
    media_container_items_any(json, &["Device", "devices"])
        .into_iter()
        .filter(|device| {
            device
                .get("provides")
                .and_then(Value::as_str)
                .is_some_and(|provides| provides.split(',').any(|part| part.trim() == "server"))
        })
        .flat_map(|device| {
            let name =
                json_string(device, &["name", "Name"]).unwrap_or_else(|| "Plex Server".to_owned());
            let machine_identifier = json_string(
                device,
                &["clientIdentifier", "machineIdentifier", "MachineIdentifier"],
            )
            .unwrap_or_default();
            let owned = json_bool(device, &["owned"]).unwrap_or(true);
            let resource_access_token =
                json_string(device, &["accessToken", "token", "AccessToken"]).unwrap_or_default();
            let connections = media_container_items_any(device, &["Connection", "connections"]);
            let has_local_connection = owned
                && connections.iter().any(|connection| {
                    json_bool(connection, &["local"]).unwrap_or(false)
                        || connection_uri(connection)
                            .is_some_and(|uri| uri_host_looks_private(&uri))
                });
            connections
                .into_iter()
                .filter_map(move |connection| {
                    let uri = connection_uri(connection)?;
                    if !owned && json_bool(connection, &["local"]).unwrap_or(false) {
                        return None;
                    }
                    let connection_kind = server_connection_kind(connection, &uri);
                    let access_token =
                        json_string(connection, &["accessToken", "token", "AccessToken"])
                            .unwrap_or_else(|| resource_access_token.clone());
                    if uri.trim().is_empty() || access_token.trim().is_empty() {
                        return None;
                    }
                    Some(PlexServerConnection {
                        name: name.clone(),
                        machine_identifier: machine_identifier.clone(),
                        uri,
                        access_token: access_token.into(),
                        owned,
                        has_local_connection,
                        connection_kind,
                    })
                })
                .min_by_key(server_connection_rank)
        })
        .collect()
}

fn parse_server_machine_identifier_response(json: &Value) -> PlexResult<String> {
    let container = json.get("MediaContainer").unwrap_or(json);
    json_string(
        container,
        &[
            "machineIdentifier",
            "MachineIdentifier",
            "clientIdentifier",
            "ClientIdentifier",
        ],
    )
    .filter(|value| !value.trim().is_empty())
    .ok_or_else(|| {
        PlexError::InvalidResponse(
            "server identity response did not include a machine identifier".to_owned(),
        )
    })
}

fn merge_server_connections(
    servers: &mut Vec<PlexServerConnection>,
    additional: Vec<PlexServerConnection>,
) {
    for server in additional {
        if let Some(existing) = servers
            .iter_mut()
            .find(|existing| server_identity_key(existing) == server_identity_key(&server))
        {
            let has_local_connection = existing.has_local_connection || server.has_local_connection;
            let owned = existing.owned || server.owned;
            if server_connection_rank(&server) < server_connection_rank(existing) {
                *existing = server;
            }
            existing.has_local_connection = has_local_connection;
            existing.owned = owned;
        } else {
            servers.push(server);
        }
    }
}

fn server_identity_key(server: &PlexServerConnection) -> String {
    let machine_identifier = server.machine_identifier.trim();
    if !machine_identifier.is_empty() {
        format!("machine:{machine_identifier}")
    } else {
        format!("name:{}", server.name.trim())
    }
}

fn connection_uri(connection: &Value) -> Option<String> {
    if let Some(uri) = json_string(connection, &["uri"]) {
        return Some(uri);
    }
    let address = json_string(connection, &["address"])?;
    if address.contains("://") {
        return Some(address);
    }
    let protocol = json_string(connection, &["protocol"]).unwrap_or_else(|| "http".to_owned());
    let port = json_string(connection, &["port"]).unwrap_or_else(|| "32400".to_owned());
    Some(format!("{protocol}://{address}:{port}"))
}

fn server_connection_kind(connection: &Value, uri: &str) -> PlexServerConnectionKind {
    if json_bool(connection, &["local"]).unwrap_or(false) || uri_host_looks_private(uri) {
        PlexServerConnectionKind::Local
    } else if uri_uses_port(uri, "8443") {
        PlexServerConnectionKind::Relay
    } else {
        PlexServerConnectionKind::Remote
    }
}

pub fn plex_server_connection_kind_from_uri(uri: &str) -> PlexServerConnectionKind {
    if uri_host_looks_private(uri) {
        PlexServerConnectionKind::Local
    } else if uri_uses_port(uri, "8443") {
        PlexServerConnectionKind::Relay
    } else {
        PlexServerConnectionKind::Remote
    }
}

fn server_connection_rank(server: &PlexServerConnection) -> (u8, u8, u8, String) {
    let connection_class = match server.connection_kind {
        PlexServerConnectionKind::Remote => 0,
        PlexServerConnectionKind::Local => 1,
        PlexServerConnectionKind::Relay => 2,
    };
    let scheme_penalty = if server.uri.starts_with("https://") {
        0
    } else {
        1
    };
    let port_penalty = if uri_uses_port(&server.uri, "32400") {
        0
    } else {
        1
    };
    (
        connection_class,
        scheme_penalty,
        port_penalty,
        server.uri.clone(),
    )
}

fn uri_uses_port(uri: &str, port: &str) -> bool {
    uri_authority(uri)
        .and_then(|authority| authority.rsplit_once(':').map(|(_, value)| value == port))
        .unwrap_or(false)
}

fn uri_host_looks_private(uri: &str) -> bool {
    let Some(host_without_port) = uri_host_without_port(uri) else {
        return false;
    };
    let host_without_port = host_without_port.trim();
    if let Ok(ip) = host_without_port.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(address) => is_private_or_loopback_ipv4(address.octets()),
            std::net::IpAddr::V6(address) => is_private_or_loopback_ipv6(address),
        };
    }
    host_without_port.eq_ignore_ascii_case("localhost")
        || parse_ipv4_octets(host_without_port)
            .or_else(|| {
                host_without_port
                    .split('.')
                    .next()
                    .map(|label| label.replace('-', "."))
                    .and_then(|label| parse_ipv4_octets(&label))
            })
            .is_some_and(is_private_or_loopback_ipv4)
}

fn uri_host_without_port(uri: &str) -> Option<&str> {
    let authority = uri_authority(uri)?;
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    if let Some(bracketed) = host.strip_prefix('[') {
        return bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .filter(|value| !value.is_empty());
    }
    Some(host.rsplit_once(':').map(|(host, _)| host).unwrap_or(host))
        .filter(|value| !value.is_empty())
}

fn uri_authority(uri: &str) -> Option<&str> {
    let after_scheme = uri.split_once("://").map(|(_, rest)| rest).unwrap_or(uri);
    after_scheme
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
}

fn parse_ipv4_octets(value: &str) -> Option<[u8; 4]> {
    let mut octets = [0_u8; 4];
    let mut parts = value.split('.');
    for octet in &mut octets {
        *octet = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(octets)
}

fn is_private_or_loopback_ipv4(octets: [u8; 4]) -> bool {
    matches!(
        octets,
        [10, _, _, _] | [127, _, _, _] | [169, 254, _, _] | [172, 16..=31, _, _] | [192, 168, _, _]
    )
}

fn is_private_or_loopback_ipv6(address: std::net::Ipv6Addr) -> bool {
    let first_segment = address.segments()[0];
    address.is_loopback() || first_segment & 0xfe00 == 0xfc00 || first_segment & 0xffc0 == 0xfe80
}

fn parse_search_response(json: &Value) -> Vec<PlexMediaSearchResult> {
    let mut output = Vec::new();
    collect_search_results(json, &mut output);
    output
}

fn parse_library_sections_response(json: &Value) -> Vec<PlexLibrarySection> {
    media_container_items_any(json, &["Directory", "directories"])
        .into_iter()
        .filter_map(|directory| {
            let key = json_string(directory, &["key"])?;
            if key.trim().is_empty() {
                return None;
            }
            Some(PlexLibrarySection {
                key,
                library_type: json_string(directory, &["type"]).unwrap_or_default(),
            })
        })
        .collect()
}

fn library_section_media_type_filters(library_type: &str) -> Vec<&'static str> {
    match library_type.trim().to_ascii_lowercase().as_str() {
        "movie" => vec!["1"],
        "show" => vec!["4"],
        "artist" | "photo" => Vec::new(),
        _ => vec!["1", "4"],
    }
}

fn library_section_text_query_filters(media_type: &str) -> Vec<&'static str> {
    let mut filters = vec!["title"];
    if media_type == "4" {
        filters.push("show.title");
    }
    filters.push("file");
    filters
}

fn filter_media_search_results_by_query(
    results: Vec<PlexMediaSearchResult>,
    query: &str,
) -> Vec<PlexMediaSearchResult> {
    results
        .into_iter()
        .filter(|result| media_search_result_matches_query(result, query))
        .collect()
}

fn media_search_result_matches_query(result: &PlexMediaSearchResult, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    let raw_query = query.to_lowercase();
    let query_terms = normalized_search_terms(query);
    if query_terms.is_empty() {
        return true;
    }

    if text_matches_search_query(&result.title, &raw_query, &query_terms) {
        return true;
    }
    if result
        .parent_title
        .as_deref()
        .is_some_and(|value| text_matches_search_query(value, &raw_query, &query_terms))
    {
        return true;
    }
    if result
        .grandparent_title
        .as_deref()
        .is_some_and(|value| text_matches_search_query(value, &raw_query, &query_terms))
    {
        return true;
    }

    result
        .file_paths
        .iter()
        .any(|value| text_matches_search_query(value, &raw_query, &query_terms))
}

fn text_matches_search_query(value: &str, raw_query: &str, query_terms: &[String]) -> bool {
    value.to_lowercase().contains(raw_query) || {
        let normalized = normalized_search_text(value);
        query_terms.iter().all(|term| normalized.contains(term))
    }
}

fn normalized_search_terms(value: &str) -> Vec<String> {
    normalized_search_text(value)
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect()
}

fn normalized_search_text(value: &str) -> String {
    let mut output = String::new();
    let mut last_was_space = true;
    for ch in value.chars() {
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() {
                output.push(lower);
                last_was_space = false;
            } else if !last_was_space {
                output.push(' ');
                last_was_space = true;
            }
        }
    }
    output.trim().to_owned()
}

fn merge_media_search_results(
    output: &mut Vec<PlexMediaSearchResult>,
    results: Vec<PlexMediaSearchResult>,
) {
    for mut result in results {
        if let Some(existing) = output
            .iter_mut()
            .find(|existing| existing.rating_key == result.rating_key)
        {
            if existing.parent_title.is_none() {
                existing.parent_title = result.parent_title.take();
            }
            if existing.grandparent_title.is_none() {
                existing.grandparent_title = result.grandparent_title.take();
            }
            existing.file_paths.append(&mut result.file_paths);
            existing.file_paths.sort();
            existing.file_paths.dedup();
        } else {
            result.file_paths.sort();
            result.file_paths.dedup();
            output.push(result);
        }
    }
}

fn collect_search_results(value: &Value, output: &mut Vec<PlexMediaSearchResult>) {
    match value {
        Value::Object(map) => {
            if let Some(rating_key) = map
                .get("ratingKey")
                .or_else(|| map.get("key"))
                .and_then(value_as_string)
                .filter(|value| !value.trim().is_empty())
                && let Some(type_name) = map
                    .get("type")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
            {
                let title = map
                    .get("title")
                    .or_else(|| map.get("grandparentTitle"))
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled")
                    .to_owned();
                let mut file_paths = Vec::new();
                collect_file_paths(value, &mut file_paths);
                file_paths.sort();
                file_paths.dedup();
                output.push(PlexMediaSearchResult {
                    rating_key,
                    title,
                    parent_title: json_string(value, &["parentTitle"]),
                    grandparent_title: json_string(value, &["grandparentTitle"]),
                    media_type: PlexMediaType::from_plex_type(type_name),
                    duration_millis: map.get("duration").and_then(value_as_u64),
                    file_paths,
                });
            }
            for child in map.values() {
                collect_search_results(child, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_search_results(item, output);
            }
        }
        _ => {}
    }
}

fn collect_file_paths(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(file) = map
                .get("file")
                .and_then(value_as_string)
                .filter(|value| !value.trim().is_empty())
            {
                output.push(file);
            }
            for child in map.values() {
                collect_file_paths(child, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_file_paths(item, output);
            }
        }
        _ => {}
    }
}

fn media_container_items_any<'a>(json: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    let container = json.get("MediaContainer").unwrap_or(json);
    if let Value::Array(items) = container {
        return items.iter().collect();
    }
    for key in keys {
        match container.get(*key) {
            Some(Value::Array(items)) => return items.iter().collect(),
            Some(item @ Value::Object(_)) => return vec![item],
            _ => {}
        }
    }
    Vec::new()
}

fn json_string(json: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(value_as_string))
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.to_owned()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn json_bool(json: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(value_as_bool))
}

fn value_as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(number) => number.as_i64().map(|value| value != 0),
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn plex_auth_url(auth_app_url: &str, client_identifier: &str, code: &str, product: &str) -> String {
    format!(
        "{}#?clientID={}&code={}&context%5Bdevice%5D%5Bproduct%5D={}",
        auth_app_url,
        percent_encode_fragment_value(client_identifier),
        percent_encode_fragment_value(code),
        percent_encode_fragment_value(product)
    )
}

fn percent_encode_fragment_value(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

fn percent_encode_path_segment(value: &str) -> String {
    percent_encode_fragment_value(value)
}

fn percent_encode_query_value(value: &str) -> String {
    percent_encode_fragment_value(value)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
        {
            decoded.push((high << 4) | low);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn parse_query_pairs_lossy(query: &str) -> Vec<(String, Option<String>)> {
    query
        .split('&')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (
                percent_decode_lossy(key),
                (!value.is_empty()).then(|| percent_decode_lossy(value)),
            )
        })
        .collect()
}

fn non_empty_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn plex_playlist_media_type_from_value(value: &str) -> Option<PlexMediaType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "movie" => Some(PlexMediaType::Movie),
        "episode" => Some(PlexMediaType::Episode),
        _ => None,
    }
}

pub fn redact_plex_token(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    loop {
        let Some((match_start, pattern_len, header_style)) = find_plex_token_pattern(remaining)
        else {
            output.push_str(remaining);
            break;
        };
        output.push_str(&remaining[..match_start + pattern_len]);
        let mut value_start = match_start + pattern_len;
        if header_style {
            while remaining
                .as_bytes()
                .get(value_start)
                .is_some_and(u8::is_ascii_whitespace)
            {
                output.push(remaining.as_bytes()[value_start] as char);
                value_start += 1;
            }
        }
        output.push_str("<redacted>");
        let value_end = plex_token_value_end(&remaining[value_start..]) + value_start;
        remaining = &remaining[value_end..];
    }
    output
}

fn find_plex_token_pattern(value: &str) -> Option<(usize, usize, bool)> {
    let lower = value.to_ascii_lowercase();
    [
        ("x-plex-token=", false),
        ("x-plex-token%3d", false),
        ("x%2dplex%2dtoken=", false),
        ("x%2dplex%2dtoken%3d", false),
        ("x-plex-token:", true),
        ("x-plex-token%3a", true),
        ("x%2dplex%2dtoken:", true),
        ("x%2dplex%2dtoken%3a", true),
    ]
    .into_iter()
    .filter_map(|(pattern, header_style)| {
        lower
            .find(pattern)
            .map(|index| (index, pattern.len(), header_style))
    })
    .min_by_key(|(index, _, _)| *index)
}

fn plex_token_value_end(value: &str) -> usize {
    value
        .char_indices()
        .find_map(|(index, ch)| {
            matches!(
                ch,
                '&' | '"' | '\'' | ')' | '(' | '[' | ']' | '{' | '}' | '\r' | '\n' | ' ' | '\t'
            )
            .then_some(index)
        })
        .unwrap_or(value.len())
}

fn insert_header_value(
    headers: &mut reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
    value: &str,
) {
    if let Ok(value) = reqwest::header::HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn push_query_param(query: &mut Vec<(String, String)>, name: &str, value: &str) {
    if !value.trim().is_empty() {
        query.push((name.to_owned(), value.to_owned()));
    }
}

fn plex_client_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "ios" => "iOS",
        "android" => "Android",
        "linux" => "Linux",
        "freebsd" => "FreeBSD",
        "openbsd" => "OpenBSD",
        "netbsd" => "NetBSD",
        "dragonfly" => "DragonFly BSD",
        "solaris" => "Solaris",
        _ => std::env::consts::OS,
    }
}

fn normalize_path_key(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

fn media_file_name_for_file(file: &LocalFileUpdate) -> Option<String> {
    file.path
        .as_deref()
        .and_then(path_file_name)
        .or_else(|| path_file_name(&file.name))
        .filter(|value| !value.trim().is_empty())
}

fn path_file_name(path: &str) -> Option<String> {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_file_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn local_file_path_matches_plex_path(file: &LocalFileUpdate, plex_path: &str) -> bool {
    let Some(local_path) = file
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return false;
    };
    let local = normalize_path_key(local_path);
    let plex = normalize_path_key(plex_path);
    if local.is_empty() || plex.is_empty() {
        return false;
    }
    if local == plex {
        return true;
    }

    let local_suffixes = normalized_path_suffixes(&local);
    let plex_suffixes = normalized_path_suffixes(&plex);
    local_suffixes.iter().any(|local_suffix| {
        local_suffix.contains('/')
            && plex_suffixes
                .iter()
                .any(|plex_suffix| plex_suffix == local_suffix)
    })
}

fn normalized_path_suffixes(normalized_path: &str) -> Vec<String> {
    let mut output = vec![normalized_path.to_owned()];
    if normalized_path.as_bytes().get(1) == Some(&b':')
        && normalized_path.as_bytes().get(2) == Some(&b'/')
    {
        output.push(normalized_path[3..].to_owned());
    }
    let parts = normalized_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    for index in 1..parts.len() {
        output.push(parts[index..].join("/"));
    }
    output.sort();
    output.dedup();
    output
}

fn normalized_cache_scope_value(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}

fn normalized_title_stem(name: &str) -> String {
    let file_name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(name);
    let mut output = String::new();
    let mut bracket_depth = 0_usize;
    for ch in file_name.chars() {
        match ch {
            '[' | '(' | '{' => bracket_depth = bracket_depth.saturating_add(1),
            ']' | ')' | '}' => bracket_depth = bracket_depth.saturating_sub(1),
            _ if bracket_depth > 0 => {}
            _ if ch.is_ascii_alphanumeric() => output.push(ch.to_ascii_lowercase()),
            _ => output.push(' '),
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn seconds_to_millis(seconds: f64) -> Option<u64> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

fn ensure_rustls_crypto_provider() {
    RUSTLS_PROVIDER_INIT.get_or_init(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        io::{Read, Write},
        net::TcpListener,
        rc::Rc,
        sync::mpsc,
        thread,
        time::{Duration, SystemTime},
    };

    use super::*;

    #[test]
    fn plex_auth_poll_result_debug_redacts_auth_token() {
        let result = PlexAuthPollResult {
            auth_token: Some("poll-result-secret".into()),
            expires_at: None,
        };

        let debug = format!("{result:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("poll-result-secret"));
    }

    #[test]
    fn plex_client_config_debug_redacts_all_tokens() {
        let config = PlexClientConfig {
            user_token: Some("user-token-secret".into()),
            selected_server_token: Some("server-token-secret".into()),
            ..PlexClientConfig::default()
        };

        let debug = format!("{config:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("user-token-secret"));
        assert!(!debug.contains("server-token-secret"));
    }

    #[test]
    fn plex_server_connection_debug_redacts_access_token() {
        let server = PlexServerConnection {
            name: "Test Server".to_owned(),
            machine_identifier: "test-server".to_owned(),
            uri: "https://plex.invalid".to_owned(),
            access_token: "access-token-secret".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        };

        let debug = format!("{server:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("access-token-secret"));
    }

    #[derive(Clone, Default)]
    struct FakeTransport {
        searches: Rc<RefCell<Vec<String>>>,
        search_results: Rc<RefCell<Vec<PlexMediaSearchResult>>>,
        file_searches: Rc<RefCell<Vec<String>>>,
        file_search_results: Rc<RefCell<Vec<PlexMediaSearchResult>>>,
        discoveries: Rc<RefCell<Vec<String>>>,
        discovered_servers: Rc<RefCell<Vec<PlexServerConnection>>>,
        metadata_lookups: Rc<RefCell<Vec<String>>>,
        metadata_results: Rc<RefCell<BTreeMap<String, PlexMediaMetadata>>>,
        machine_identifier_lookups: Rc<RefCell<Vec<String>>>,
        machine_identifier_result: Rc<RefCell<String>>,
        stream_parts: Rc<RefCell<Vec<String>>>,
        stream_urls: Rc<RefCell<Vec<String>>>,
        stream_tokens: Rc<RefCell<Vec<String>>>,
        reports: Rc<RefCell<Vec<PlexTimelineReport>>>,
    }

    impl PlexSyncTransport for FakeTransport {
        fn search_media(
            &self,
            _server_url: &str,
            _token: &str,
            query: &str,
        ) -> PlexResult<Vec<PlexMediaSearchResult>> {
            self.searches.borrow_mut().push(query.to_owned());
            Ok(self.search_results.borrow().clone())
        }

        fn search_media_by_file_name(
            &self,
            _server_url: &str,
            _token: &str,
            file_name: &str,
        ) -> PlexResult<Vec<PlexMediaSearchResult>> {
            self.file_searches.borrow_mut().push(file_name.to_owned());
            Ok(self.file_search_results.borrow().clone())
        }

        fn report_timeline(
            &self,
            _server_url: &str,
            _token: &str,
            report: &PlexTimelineReport,
        ) -> PlexResult<()> {
            self.reports.borrow_mut().push(report.clone());
            Ok(())
        }
    }

    impl PlexMetadataTransport for FakeTransport {
        fn metadata_by_rating_key(
            &self,
            server_url: &str,
            token: &str,
            rating_key: &str,
        ) -> PlexResult<PlexMediaMetadata> {
            self.metadata_lookups
                .borrow_mut()
                .push(format!("{server_url}|{token}|{rating_key}"));
            self.metadata_results
                .borrow()
                .get(rating_key)
                .cloned()
                .ok_or_else(|| {
                    PlexError::InvalidResponse(format!(
                        "metadata test fixture missing rating key {rating_key}"
                    ))
                })
        }

        fn build_part_stream_url(
            &self,
            server_url: &str,
            token: &str,
            part: &PlexPlayablePart,
        ) -> PlexResult<SecretPlexPlaybackUrl> {
            self.stream_parts.borrow_mut().push(part.key.clone());
            self.stream_urls.borrow_mut().push(server_url.to_owned());
            self.stream_tokens.borrow_mut().push(token.to_owned());
            Ok(SecretPlexPlaybackUrl::new(format!(
                "{server_url}{}?X-Plex-Token={token}",
                part.key
            )))
        }

        fn server_machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String> {
            self.machine_identifier_lookups
                .borrow_mut()
                .push(format!("{server_url}|{token}"));
            let machine_identifier = self.machine_identifier_result.borrow().clone();
            if machine_identifier.trim().is_empty() {
                return Err(PlexError::InvalidResponse(
                    "metadata test fixture missing machine identifier".to_owned(),
                ));
            }
            Ok(machine_identifier)
        }
    }

    impl PlexServerDiscoveryTransport for FakeTransport {
        fn discover_servers(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>> {
            self.discoveries.borrow_mut().push(user_token.to_owned());
            Ok(self.discovered_servers.borrow().clone())
        }
    }

    fn configured_engine(transport: FakeTransport) -> PlexSyncEngine<FakeTransport> {
        PlexSyncEngine::new(
            PlexClientConfig {
                enabled: true,
                selected_server_id: Some("abc123machine".to_owned()),
                selected_server_url: Some("http://plex.local:32400".to_owned()),
                selected_server_token: Some("server-token".into()),
                ..PlexClientConfig::default()
            },
            transport,
            PlexMatchCache::default(),
        )
    }

    fn movie_file() -> LocalFileUpdate {
        LocalFileUpdate::new("Example.Movie.2024.mkv")
            .with_duration_seconds(7200.0)
            .with_path("C:/Media/Example.Movie.2024.mkv")
    }

    fn example_metadata() -> PlexMediaMetadata {
        PlexMediaMetadata {
            rating_key: "456".to_owned(),
            title: "Example".to_owned(),
            media_type: PlexMediaType::Movie,
            duration_millis: Some(7_200_000),
            parts: vec![PlexPlayablePart {
                id: "part-1".to_owned(),
                key: "/library/parts/1/file.mkv".to_owned(),
                file_name: Some("Example.mkv".to_owned()),
                duration_millis: Some(7_200_000),
                size_bytes: Some(123_456),
                container: Some("mkv".to_owned()),
            }],
        }
    }

    fn serve_plex_json_responses(responses: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Plex test listener should bind");
        let address = listener
            .local_addr()
            .expect("Plex test listener should expose its address");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for body in responses {
                let (mut stream, _) = listener.accept().expect("Plex test server should accept");
                let mut buffer = [0_u8; 8192];
                let read = stream
                    .read(&mut buffer)
                    .expect("Plex test server should read request");
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                tx.send(request)
                    .expect("Plex test server should send captured request");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("Plex test server should write response");
            }
        });
        (format!("http://{address}"), rx)
    }

    #[test]
    fn plex_error_display_redacts_tokens() {
        let http_error = PlexError::Http(
            "GET /library/parts/1/file.mkv?X-Plex-Token=secret-token failed".to_owned(),
        )
        .to_string();
        assert!(http_error.contains("X-Plex-Token=<redacted>"));
        assert!(!http_error.contains("secret-token"));

        let invalid_response = PlexError::InvalidResponse(
            "unexpected response with x-plex-token: other-secret".to_owned(),
        )
        .to_string();
        assert!(invalid_response.contains("x-plex-token: <redacted>"));
        assert!(!invalid_response.contains("other-secret"));
    }

    #[test]
    fn plex_playlist_uri_round_trip() {
        let uri = PlexPlaylistUri {
            machine_identifier: "abc123machine".to_owned(),
            rating_key: "456".to_owned(),
            title: Some("Example Movie".to_owned()),
            file_name: Some("Example Movie.mkv".to_owned()),
            duration_millis: Some(7_200_000),
            size_bytes: Some(123_456_789),
            media_type: Some(PlexMediaType::Movie),
        };

        let formatted = format_plex_playlist_uri(&uri);
        let parsed = parse_plex_playlist_uri(&formatted).expect("URI should parse");

        assert_eq!(
            formatted,
            "plex://abc123machine/metadata/456?title=Example%20Movie&file=Example%20Movie.mkv&duration=7200000&size=123456789&type=movie"
        );
        assert_eq!(parsed, uri);
    }

    #[test]
    fn plex_playlist_uri_valid_episode_uri() {
        let parsed =
            parse_plex_playlist_uri("plex://machine/metadata/episode-7?title=Pilot&type=episode")
                .expect("episode URI should parse");

        assert_eq!(parsed.machine_identifier, "machine");
        assert_eq!(parsed.rating_key, "episode-7");
        assert_eq!(parsed.media_type, Some(PlexMediaType::Episode));
    }

    #[test]
    fn plex_playlist_uri_rejects_missing_machine_identifier() {
        let error = parse_plex_playlist_uri("plex:///metadata/456")
            .expect_err("machine id is required")
            .to_string();

        assert!(error.contains("machine identifier"));
    }

    #[test]
    fn plex_playlist_uri_rejects_missing_rating_key() {
        let error = parse_plex_playlist_uri("plex://machine/metadata/")
            .expect_err("rating key is required")
            .to_string();

        assert!(error.contains("rating key"));
    }

    #[test]
    fn plex_playlist_uri_decodes_title_and_file_and_ignores_unknown_keys() {
        let parsed = parse_plex_playlist_uri(
            "plex://machine/metadata/456?title=Example%20Movie&file=Example%20Movie.mkv&ignored=value",
        )
        .expect("encoded hints should parse");

        assert_eq!(parsed.title.as_deref(), Some("Example Movie"));
        assert_eq!(parsed.file_name.as_deref(), Some("Example Movie.mkv"));
    }

    #[test]
    fn plex_playlist_uri_rejects_tokens() {
        let error = parse_plex_playlist_uri("plex://machine/metadata/456?X-Plex-Token=secret")
            .expect_err("token query should be rejected")
            .to_string();

        assert!(error.contains("token"));
        assert!(!error.contains("secret"));
    }

    #[test]
    fn parse_metadata_extracts_video_part_key() {
        let json: Value = serde_json::json!({
            "MediaContainer": {
                "Metadata": [{
                    "ratingKey": "456",
                    "type": "movie",
                    "title": "Example",
                    "duration": 7200000,
                    "Media": [{
                        "container": "mkv",
                        "Part": [{
                            "id": "99",
                            "key": "/library/parts/99/file.mkv",
                            "file": "E:/Movies/Example.mkv",
                            "duration": 7200000,
                            "size": 123456
                        }]
                    }]
                }]
            }
        });

        let metadata = parse_metadata_response(&json, "456").expect("metadata should parse");

        assert_eq!(metadata.rating_key, "456");
        assert_eq!(metadata.media_type, PlexMediaType::Movie);
        assert_eq!(metadata.parts.len(), 1);
        assert_eq!(metadata.parts[0].key, "/library/parts/99/file.mkv");
        assert_eq!(metadata.parts[0].file_name.as_deref(), Some("Example.mkv"));
    }

    #[test]
    fn metadata_resolution_rejects_audio_or_other_type() {
        let metadata = PlexMediaMetadata {
            rating_key: "track-1".to_owned(),
            title: "Song".to_owned(),
            media_type: PlexMediaType::Other,
            duration_millis: Some(180_000),
            parts: vec![PlexPlayablePart {
                id: "audio".to_owned(),
                key: "/library/parts/audio.mp3".to_owned(),
                file_name: Some("song.mp3".to_owned()),
                duration_millis: Some(180_000),
                size_bytes: Some(1000),
                container: Some("mp3".to_owned()),
            }],
        };

        let error = choose_playable_part(&metadata, Some(180_000))
            .expect_err("non-video metadata should be rejected")
            .to_string();

        assert!(error.contains("not playable video"));
    }

    #[test]
    fn stream_url_appends_token_but_debug_redacts_it() {
        let client = PlexHttpClient::new("stream-url-test").expect("Plex client should construct");
        let url = client
            .build_part_stream_url(
                "http://plex.local:32400",
                "secret-token",
                &PlexPlayablePart {
                    id: "1".to_owned(),
                    key: "/library/parts/1/file.mkv".to_owned(),
                    file_name: Some("Example.mkv".to_owned()),
                    duration_millis: None,
                    size_bytes: None,
                    container: None,
                },
            )
            .expect("stream URL should build");

        assert!(url.as_str().contains("X-Plex-Token=secret-token"));
        let debug = format!("{url:?}");
        assert!(debug.contains("X-Plex-Token=<redacted>"));
        assert!(!debug.contains("secret-token"));
    }

    #[test]
    fn resolve_stream_target_by_plex_uri() {
        let transport = FakeTransport::default();
        transport
            .metadata_results
            .borrow_mut()
            .insert("456".to_owned(), example_metadata());
        let mut resolver = PlexMediaResolver::new(
            PlexClientConfig {
                streaming_enabled: true,
                selected_server_id: Some("abc123machine".to_owned()),
                selected_server_url: Some("http://plex.local:32400".to_owned()),
                selected_server_token: Some("server-token".into()),
                ..PlexClientConfig::default()
            },
            transport.clone(),
            PlexMatchCache::default(),
        );

        let target = resolver
            .resolve_stream_target(
                "plex://abc123machine/metadata/456?title=Example&file=Example.mkv&duration=7200000&type=movie",
                SystemTime::UNIX_EPOCH,
            )
            .expect("stream target should resolve")
            .expect("Plex URI should resolve to stream target");

        assert_eq!(target.matched_item.rating_key, "456");
        assert_eq!(target.logical_file.name, "Example.mkv");
        assert_eq!(
            target.logical_file.path.as_deref(),
            Some(
                "plex://abc123machine/metadata/456?title=Example&file=Example.mkv&duration=7200000&size=123456&type=movie"
            )
        );
        assert_eq!(
            transport.stream_parts.borrow().as_slice(),
            &["/library/parts/1/file.mkv"]
        );
        assert!(target.playback_url.as_str().contains("server-token"));
    }

    #[test]
    fn resolve_stream_target_uses_accessible_server_matching_playlist_uri() {
        let transport = FakeTransport::default();
        transport
            .metadata_results
            .borrow_mut()
            .insert("456".to_owned(), example_metadata());
        transport
            .discovered_servers
            .borrow_mut()
            .extend([PlexServerConnection {
                name: "Shared Server".to_owned(),
                machine_identifier: "abc123machine".to_owned(),
                uri: "http://shared.plex:32400".to_owned(),
                access_token: "shared-server-token".into(),
                owned: false,
                has_local_connection: false,
                connection_kind: PlexServerConnectionKind::Remote,
            }]);
        let mut resolver = PlexMediaResolver::new(
            PlexClientConfig {
                streaming_enabled: true,
                user_token: Some("user-token".into()),
                selected_server_id: Some("other-machine".to_owned()),
                selected_server_url: Some("http://plex.local:32400".to_owned()),
                selected_server_token: Some("server-token".into()),
                ..PlexClientConfig::default()
            },
            transport,
            PlexMatchCache::default(),
        );

        let target = resolver
            .resolve_stream_target(
                "plex://abc123machine/metadata/456?title=Example&file=Example.mkv",
                SystemTime::UNIX_EPOCH,
            )
            .expect("server mismatch should resolve through accessible shared server")
            .expect("Plex URI should resolve to stream target");

        let (_, transport, _) = resolver.into_parts();

        assert_eq!(target.matched_item.rating_key, "456");
        assert_eq!(transport.discoveries.borrow().as_slice(), &["user-token"]);
        assert_eq!(
            transport.metadata_lookups.borrow().as_slice(),
            &["http://shared.plex:32400|shared-server-token|456"]
        );
        assert_eq!(
            transport.stream_urls.borrow().as_slice(),
            &["http://shared.plex:32400"]
        );
        assert_eq!(
            transport.stream_tokens.borrow().as_slice(),
            &["shared-server-token"]
        );
        assert!(target.playback_url.as_str().contains("shared-server-token"));
        assert!(
            !target
                .playback_url
                .as_str()
                .contains("X-Plex-Token=server-token")
        );
    }

    #[test]
    fn resolve_stream_target_for_playlist_uri_does_not_require_selected_server() {
        let transport = FakeTransport::default();
        transport
            .metadata_results
            .borrow_mut()
            .insert("456".to_owned(), example_metadata());
        transport
            .discovered_servers
            .borrow_mut()
            .extend([PlexServerConnection {
                name: "Shared Server".to_owned(),
                machine_identifier: "abc123machine".to_owned(),
                uri: "http://shared.plex:32400".to_owned(),
                access_token: "shared-server-token".into(),
                owned: false,
                has_local_connection: false,
                connection_kind: PlexServerConnectionKind::Remote,
            }]);
        let mut resolver = PlexMediaResolver::new(
            PlexClientConfig {
                streaming_enabled: true,
                user_token: Some("user-token".into()),
                ..PlexClientConfig::default()
            },
            transport,
            PlexMatchCache::default(),
        );

        let target = resolver
            .resolve_stream_target(
                "plex://abc123machine/metadata/456?title=Example&file=Example.mkv",
                SystemTime::UNIX_EPOCH,
            )
            .expect("Plex URI should resolve by accessible shared server")
            .expect("Plex URI should resolve to stream target");
        let (_, transport, _) = resolver.into_parts();

        assert_eq!(target.matched_item.rating_key, "456");
        assert_eq!(transport.discoveries.borrow().as_slice(), &["user-token"]);
        assert_eq!(
            transport.stream_tokens.borrow().as_slice(),
            &["shared-server-token"]
        );
    }

    #[test]
    fn ambiguous_part_selection_fails_closed() {
        let metadata = PlexMediaMetadata {
            rating_key: "456".to_owned(),
            title: "Example".to_owned(),
            media_type: PlexMediaType::Movie,
            duration_millis: Some(7_200_000),
            parts: vec![
                PlexPlayablePart {
                    id: "a".to_owned(),
                    key: "/library/parts/a.mkv".to_owned(),
                    file_name: Some("a.mkv".to_owned()),
                    duration_millis: Some(7_200_000),
                    size_bytes: None,
                    container: Some("mkv".to_owned()),
                },
                PlexPlayablePart {
                    id: "b".to_owned(),
                    key: "/library/parts/b.mkv".to_owned(),
                    file_name: Some("b.mkv".to_owned()),
                    duration_millis: Some(7_200_000),
                    size_bytes: None,
                    container: Some("mkv".to_owned()),
                },
            ],
        };

        let error = choose_playable_part(&metadata, Some(7_200_000))
            .expect_err("equal candidates should be ambiguous")
            .to_string();

        assert!(error.contains("ambiguous"));
    }

    #[test]
    fn auth_response_builds_session_and_url() {
        let json: Value = serde_json::json!({
            "id": 42,
            "code": "ABCD",
            "expiresAt": "2026-05-01T01:02:03Z"
        });

        let session =
            parse_auth_session_response(&json, "https://app.plex.tv/auth", "client id", "Product")
                .expect("auth response should parse");

        assert_eq!(session.pin_id, 42);
        assert_eq!(session.code, "ABCD");
        assert_eq!(
            session.auth_url,
            "https://app.plex.tv/auth#?clientID=client%20id&code=ABCD&context%5Bdevice%5D%5Bproduct%5D=Product"
        );
    }

    #[test]
    fn plex_headers_include_play_history_identity_fields() {
        let client = PlexHttpClient::with_base_urls(
            "https://plex.invalid",
            "https://app.plex.tv/auth",
            "history-client",
            "SorotteHistory",
        )
        .expect("Plex client should construct");

        let headers = client.plex_headers(Some("server-token"));

        assert_eq!(
            headers
                .get("x-plex-client-identifier")
                .and_then(|value| value.to_str().ok()),
            Some("history-client")
        );
        assert_eq!(
            headers
                .get("x-plex-product")
                .and_then(|value| value.to_str().ok()),
            Some("SorotteHistory")
        );
        assert_eq!(
            headers
                .get("x-plex-version")
                .and_then(|value| value.to_str().ok()),
            Some(DEFAULT_CLIENT_VERSION)
        );
        assert_eq!(
            headers
                .get("x-plex-platform")
                .and_then(|value| value.to_str().ok()),
            Some(plex_client_platform())
        );
        assert_eq!(
            headers
                .get("x-plex-device")
                .and_then(|value| value.to_str().ok()),
            Some(plex_client_platform())
        );
        assert_eq!(
            headers
                .get("x-plex-device-name")
                .and_then(|value| value.to_str().ok()),
            Some("SorotteHistory")
        );
        assert_eq!(
            headers
                .get("x-plex-token")
                .and_then(|value| value.to_str().ok()),
            Some("server-token")
        );
    }

    #[test]
    fn resources_response_collapses_server_connections_to_best_uri() {
        let json: Value = serde_json::json!({
            "MediaContainer": {
                "Device": [{
                    "name": "Home Plex",
                    "clientIdentifier": "machine-1",
                    "provides": "server",
                    "accessToken": "server-token",
                    "Connection": [
                        { "uri": "http://192.168.1.2:32400" },
                        { "uri": "https://remote.plex.example" }
                    ]
                }, {
                    "name": "Player",
                    "provides": "client",
                    "Connection": [{ "uri": "http://ignored" }]
                }]
            }
        });

        let servers = parse_server_resources_response(&json);

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Home Plex");
        assert_eq!(servers[0].machine_identifier, "machine-1");
        assert_eq!(servers[0].uri, "https://remote.plex.example");
        assert_eq!(servers[0].access_token.expose_secret(), "server-token");
        assert!(servers[0].owned);
        assert!(servers[0].has_local_connection);
        assert_eq!(servers[0].connection_kind, PlexServerConnectionKind::Remote);
    }

    #[test]
    fn resources_response_parses_shared_server_remote_connections() {
        let json: Value = serde_json::json!({
            "MediaContainer": {
                "Device": [{
                    "name": "Friend Plex",
                    "clientIdentifier": "shared-machine",
                    "provides": "server",
                    "owned": false,
                    "accessToken": "shared-token",
                    "connections": [
                        {
                            "uri": "http://192.168.1.20:32400",
                            "local": true
                        },
                        {
                            "uri": "https://shared.plex.direct:32400",
                            "local": false
                        }
                    ]
                }]
            }
        });

        let servers = parse_server_resources_response(&json);

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Friend Plex");
        assert_eq!(servers[0].machine_identifier, "shared-machine");
        assert_eq!(servers[0].uri, "https://shared.plex.direct:32400");
        assert_eq!(servers[0].access_token.expose_secret(), "shared-token");
        assert!(!servers[0].owned);
        assert!(!servers[0].has_local_connection);
        assert_eq!(servers[0].connection_kind, PlexServerConnectionKind::Remote);
    }

    #[test]
    fn resources_response_marks_owned_local_connections() {
        let json: Value = serde_json::json!({
            "MediaContainer": {
                "Device": [{
                    "name": "Local Plex",
                    "clientIdentifier": "local-machine",
                    "provides": "server",
                    "owned": true,
                    "accessToken": "local-token",
                    "connections": [{
                        "uri": "http://192.168.1.20:32400",
                        "local": true
                    }]
                }]
            }
        });

        let servers = parse_server_resources_response(&json);

        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "Local Plex");
        assert!(servers[0].owned);
        assert!(servers[0].has_local_connection);
        assert_eq!(servers[0].connection_kind, PlexServerConnectionKind::Local);
    }

    #[test]
    fn connection_kind_detects_case_insensitive_localhost() {
        assert_eq!(
            plex_server_connection_kind_from_uri("http://LOCALHOST:32400"),
            PlexServerConnectionKind::Local
        );
    }

    #[test]
    fn connection_kind_detects_bracketed_ipv6_local_addresses() {
        assert_eq!(
            plex_server_connection_kind_from_uri("http://[::1]:32400"),
            PlexServerConnectionKind::Local
        );
        assert_eq!(
            plex_server_connection_kind_from_uri("https://[fd12:3456:789a::1]:32400"),
            PlexServerConnectionKind::Local
        );
        assert_eq!(
            plex_server_connection_kind_from_uri("http://[fe80::1]:32400"),
            PlexServerConnectionKind::Local
        );
    }

    #[test]
    fn resources_response_parses_v2_top_level_resource_arrays() {
        let json: Value = serde_json::json!([{
            "name": "Raptor",
            "clientIdentifier": "raptor-machine",
            "provides": "server",
            "owned": true,
            "accessToken": "raptor-token",
            "connections": [{
                "uri": "https://raptor.plex.direct:32400",
                "local": false
            }]
        }, {
            "name": "zzzzzzzzzzzzzzzzzzzzzz",
            "clientIdentifier": "shared-machine",
            "provides": "server",
            "owned": false,
            "accessToken": "shared-token",
            "connections": [{
                "uri": "https://shared.plex.direct:32400",
                "local": false
            }]
        }]);

        let servers = parse_server_resources_response(&json);

        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "Raptor");
        assert_eq!(servers[0].access_token.expose_secret(), "raptor-token");
        assert!(servers[0].owned);
        assert!(!servers[0].has_local_connection);
        assert_eq!(servers[0].connection_kind, PlexServerConnectionKind::Remote);
        assert_eq!(servers[1].name, "zzzzzzzzzzzzzzzzzzzzzz");
        assert_eq!(servers[1].access_token.expose_secret(), "shared-token");
        assert!(!servers[1].owned);
        assert!(!servers[1].has_local_connection);
        assert_eq!(servers[1].connection_kind, PlexServerConnectionKind::Remote);
    }

    #[test]
    fn server_resource_merge_keeps_one_best_connection_per_machine() {
        let mut servers = vec![PlexServerConnection {
            name: "Raptor".to_owned(),
            machine_identifier: "raptor-machine".to_owned(),
            uri: "https://172-18-0-6.raptor-machine.plex.direct:32400".to_owned(),
            access_token: "raptor-token".into(),
            owned: true,
            has_local_connection: true,
            connection_kind: PlexServerConnectionKind::Local,
        }];
        merge_server_connections(
            &mut servers,
            vec![
                PlexServerConnection {
                    name: "Raptor".to_owned(),
                    machine_identifier: "raptor-machine".to_owned(),
                    uri: "https://45-56-91-134.raptor-machine.plex.direct:8443".to_owned(),
                    access_token: "raptor-token".into(),
                    owned: true,
                    has_local_connection: false,
                    connection_kind: PlexServerConnectionKind::Relay,
                },
                PlexServerConnection {
                    name: "Raptor".to_owned(),
                    machine_identifier: "raptor-machine".to_owned(),
                    uri: "https://125-209-152-187.raptor-machine.plex.direct:32400".to_owned(),
                    access_token: "raptor-token".into(),
                    owned: true,
                    has_local_connection: false,
                    connection_kind: PlexServerConnectionKind::Remote,
                },
                PlexServerConnection {
                    name: "Tower".to_owned(),
                    machine_identifier: "tower-machine".to_owned(),
                    uri: "https://180-181-237-20.tower-machine.plex.direct:32400".to_owned(),
                    access_token: "tower-token".into(),
                    owned: false,
                    has_local_connection: false,
                    connection_kind: PlexServerConnectionKind::Remote,
                },
            ],
        );

        assert_eq!(servers.len(), 2);
        assert_eq!(
            servers[0].uri,
            "https://125-209-152-187.raptor-machine.plex.direct:32400"
        );
        assert!(servers[0].has_local_connection);
        assert_eq!(servers[1].name, "Tower");
    }

    #[test]
    fn verify_server_connection_uses_server_url_and_token() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Plex test listener should bind");
        let address = listener
            .local_addr()
            .expect("Plex test listener should expose its address");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("Plex test server should accept");
            let mut buffer = [0_u8; 2048];
            let read = stream
                .read(&mut buffer)
                .expect("Plex test server should read request");
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            tx.send(request)
                .expect("Plex test server should send captured request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
                )
                .expect("Plex test server should write response");
        });
        let client = PlexHttpClient::new("verify-test").expect("Plex client should construct");
        let server = PlexServerConnection {
            name: "Raptor".to_owned(),
            machine_identifier: "raptor-machine".to_owned(),
            uri: format!("http://{address}"),
            access_token: "server-token".into(),
            owned: true,
            has_local_connection: true,
            connection_kind: PlexServerConnectionKind::Local,
        };

        client
            .verify_server_connection(&server)
            .expect("server verification should succeed");

        let request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("test should capture Plex verification request")
            .to_ascii_lowercase();
        assert!(request.starts_with("get / http/1.1"));
        assert!(request.contains("x-plex-token: server-token"));
    }

    #[test]
    fn timeline_report_sends_identity_as_headers_and_query_params() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("Plex test listener should bind");
        let address = listener
            .local_addr()
            .expect("Plex test listener should expose its address");
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("Plex test server should accept");
            let mut buffer = [0_u8; 4096];
            let read = stream
                .read(&mut buffer)
                .expect("Plex test server should read request");
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            tx.send(request)
                .expect("Plex test server should send captured request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
                )
                .expect("Plex test server should write response");
        });
        let client = PlexHttpClient::with_base_urls(
            "https://plex.invalid",
            "https://app.plex.tv/auth",
            "history-client",
            "SorotteHistory",
        )
        .expect("Plex client should construct");
        let report = PlexTimelineReport {
            rating_key: "episode-7".to_owned(),
            state: PlexTimelineState::Playing,
            time_millis: 42_000,
            duration_millis: Some(1_200_000),
        };

        client
            .report_timeline(&format!("http://{address}"), "server-token", &report)
            .expect("timeline report should succeed");

        let request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("test should capture Plex timeline request");
        let lower_request = request.to_ascii_lowercase();
        assert!(request.starts_with("GET /:/timeline?"));
        assert!(request.contains("ratingKey=episode-7"));
        assert!(request.contains("state=playing"));
        assert!(request.contains("time=42000"));
        assert!(request.contains("duration=1200000"));
        assert!(request.contains("X-Plex-Client-Identifier=history-client"));
        assert!(request.contains("X-Plex-Product=SorotteHistory"));
        assert!(request.contains("X-Plex-Version="));
        assert!(request.contains("X-Plex-Platform="));
        assert!(request.contains("X-Plex-Device="));
        assert!(request.contains("X-Plex-Device-Name=SorotteHistory"));
        assert!(lower_request.contains("x-plex-product: sorottehistory"));
        assert!(lower_request.contains("x-plex-platform:"));
        assert!(lower_request.contains("x-plex-device-name: sorottehistory"));
        assert!(lower_request.contains("x-plex-token: server-token"));
    }

    #[test]
    fn search_response_collects_video_metadata() {
        let json: Value = serde_json::json!({
            "MediaContainer": {
                "Metadata": [
                    {
                        "ratingKey": "1",
                        "type": "movie",
                        "title": "Movie",
                        "parentTitle": "Movies",
                        "grandparentTitle": "Library",
                        "duration": 60000,
                        "Media": [{ "Part": [{ "file": "E:/Movies/Movie.mkv" }] }]
                    },
                    { "ratingKey": "2", "type": "track", "title": "Song", "duration": 1000 }
                ]
            }
        });

        let results = parse_search_response(&json);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].rating_key, "1");
        assert_eq!(results[0].media_type, PlexMediaType::Movie);
        assert_eq!(results[0].parent_title.as_deref(), Some("Movies"));
        assert_eq!(results[0].grandparent_title.as_deref(), Some("Library"));
        assert_eq!(results[0].file_paths, vec!["E:/Movies/Movie.mkv"]);
    }

    #[test]
    fn selected_server_media_search_uses_video_sections_and_title_query() {
        let (server_url, rx) = serve_plex_json_responses(vec![
            serde_json::json!({
                "MediaContainer": {
                    "Directory": [
                        { "key": "1", "type": "show", "title": "Anime" },
                        { "key": "2", "type": "artist", "title": "Music" }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "14452",
                        "type": "episode",
                        "title": "Episode 11",
                        "parentTitle": "Season 4",
                        "grandparentTitle": "Re:Zero",
                        "duration": 1470058,
                        "Media": [{
                            "Part": [{
                                "file": "E:/Anime/Re Zero/Episode 11.mkv",
                                "size": 458900243
                            }]
                        }]
                    }]
                }
            })
            .to_string(),
        ]);
        let client = PlexHttpClient::new("search-test").expect("Plex client should construct");
        let config = PlexClientConfig {
            selected_server_url: Some(server_url),
            selected_server_token: Some("server-token".into()),
            ..PlexClientConfig::default()
        };

        let results = client
            .search_selected_server_media(&config, "zero", 1)
            .expect("selected server search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rating_key, "14452");
        assert_eq!(results[0].title, "Episode 11");
        assert_eq!(results[0].parent_title.as_deref(), Some("Season 4"));
        assert_eq!(results[0].grandparent_title.as_deref(), Some("Re:Zero"));
        let sections_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("sections request should be captured");
        let title_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("title request should be captured");
        assert!(sections_request.starts_with("GET /library/sections HTTP/1.1"));
        assert!(
            sections_request
                .to_ascii_lowercase()
                .contains("x-plex-token: server-token")
        );
        assert!(title_request.starts_with("GET /library/sections/1/all?"));
        assert!(title_request.contains("type=4"));
        assert!(title_request.contains("title=zero"));
        assert!(!title_request.contains("X-Plex-Token="));
    }

    #[test]
    fn selected_server_media_search_matches_episode_show_title_when_episode_title_misses() {
        let (server_url, rx) = serve_plex_json_responses(vec![
            serde_json::json!({
                "MediaContainer": {
                    "Directory": [
                        { "key": "1", "type": "show", "title": "Anime" }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": []
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "12961",
                        "type": "episode",
                        "title": "She's a Killer Queen",
                        "parentTitle": "Season 1",
                        "grandparentTitle": "Needy Girl Overdose",
                        "duration": 1439000,
                        "Media": [{
                            "Part": [{
                                "file": "E:/Anime/Needy Girl Overdose/01.mkv"
                            }]
                        }]
                    }]
                }
            })
            .to_string(),
        ]);
        let client =
            PlexHttpClient::new("show-title-search-test").expect("Plex client should construct");
        let config = PlexClientConfig {
            selected_server_url: Some(server_url),
            selected_server_token: Some("server-token".into()),
            ..PlexClientConfig::default()
        };

        let results = client
            .search_selected_server_media(&config, "Needy", 1)
            .expect("selected server search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rating_key, "12961");
        assert_eq!(
            results[0].grandparent_title.as_deref(),
            Some("Needy Girl Overdose")
        );
        let _sections_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("sections request should be captured");
        let title_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("title request should be captured");
        let show_title_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("show title request should be captured");
        assert!(title_request.contains("title=Needy"));
        assert!(show_title_request.contains("show.title=Needy"));
        assert!(!show_title_request.contains("X-Plex-Token="));
    }

    #[test]
    fn selected_server_media_search_matches_file_name_when_titles_miss() {
        let (server_url, rx) = serve_plex_json_responses(vec![
            serde_json::json!({
                "MediaContainer": {
                    "Directory": [
                        { "key": "1", "type": "show", "title": "Anime" }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": []
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": []
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "12962",
                        "type": "episode",
                        "title": "Just the Two of Us",
                        "parentTitle": "Season 1",
                        "grandparentTitle": "Unrelated Label",
                        "duration": 1439000,
                        "Media": [{
                            "Part": [{
                                "file": "E:/Anime/Needy Girl Overdose/02.mkv"
                            }]
                        }]
                    }]
                }
            })
            .to_string(),
        ]);
        let client =
            PlexHttpClient::new("file-name-search-test").expect("Plex client should construct");
        let config = PlexClientConfig {
            selected_server_url: Some(server_url),
            selected_server_token: Some("server-token".into()),
            ..PlexClientConfig::default()
        };

        let results = client
            .search_selected_server_media(&config, "Needy", 1)
            .expect("selected server search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rating_key, "12962");
        let _sections_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("sections request should be captured");
        let _title_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("title request should be captured");
        let _show_title_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("show title request should be captured");
        let file_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("file request should be captured");
        assert!(file_request.contains("file=Needy"));
        assert!(!file_request.contains("X-Plex-Token="));
    }

    #[test]
    fn selected_server_media_search_empty_query_uses_recent_paging() {
        let (server_url, rx) = serve_plex_json_responses(vec![
            serde_json::json!({
                "MediaContainer": {
                    "Directory": [
                        { "key": "movies", "type": "movie" },
                        { "key": "shows", "type": "show" }
                    ]
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "movie-1",
                        "type": "movie",
                        "title": "Recent Movie",
                        "duration": 60000
                    }]
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "episode-1",
                        "type": "episode",
                        "title": "Recent Episode",
                        "duration": 90000
                    }]
                }
            })
            .to_string(),
        ]);
        let client = PlexHttpClient::new("recent-test").expect("Plex client should construct");
        let config = PlexClientConfig {
            selected_server_url: Some(server_url),
            selected_server_token: Some("server-token".into()),
            ..PlexClientConfig::default()
        };

        let results = client
            .search_selected_server_media(&config, "", 2)
            .expect("recent selected server search should succeed");

        assert_eq!(
            results
                .iter()
                .map(|result| result.rating_key.as_str())
                .collect::<Vec<_>>(),
            vec!["movie-1", "episode-1"]
        );
        let _sections_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("sections request should be captured");
        let movie_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("movie recent request should be captured");
        let show_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("show recent request should be captured");
        assert!(movie_request.contains("sort=addedAt%3Adesc"));
        assert!(movie_request.contains("type=1"));
        assert!(show_request.contains("sort=addedAt%3Adesc"));
        assert!(show_request.contains("type=4"));
    }

    #[test]
    fn playlist_uri_for_selected_server_rating_key_fetches_missing_machine_identifier() {
        let (server_url, rx) = serve_plex_json_responses(vec![
            serde_json::json!({
                "MediaContainer": {
                    "machineIdentifier": "machine-from-root"
                }
            })
            .to_string(),
            serde_json::json!({
                "MediaContainer": {
                    "Metadata": [{
                        "ratingKey": "14452",
                        "type": "episode",
                        "title": "Episode 11",
                        "duration": 1470058,
                        "Media": [{
                            "Part": [{
                                "id": "part-1",
                                "key": "/library/parts/1/file.mkv",
                                "file": "E:/Anime/Episode 11.mkv",
                                "duration": 1470058,
                                "size": 458900243
                            }]
                        }]
                    }]
                }
            })
            .to_string(),
        ]);
        let client = PlexHttpClient::new("uri-test").expect("Plex client should construct");
        let config = PlexClientConfig {
            selected_server_url: Some(server_url),
            selected_server_token: Some("server-token".into()),
            ..PlexClientConfig::default()
        };

        let uri = client
            .playlist_uri_for_selected_server_rating_key(&config, "14452")
            .expect("playlist URI should resolve");

        assert_eq!(uri.machine_identifier, "machine-from-root");
        assert_eq!(uri.file_name.as_deref(), Some("Episode 11.mkv"));
        let formatted = format_plex_playlist_uri(&uri);
        assert!(formatted.starts_with("plex://machine-from-root/metadata/14452?"));
        assert!(!formatted.contains("token"));
        let root_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("root request should be captured");
        let metadata_request = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("metadata request should be captured");
        assert!(root_request.starts_with("GET / HTTP/1.1"));
        assert!(metadata_request.starts_with("GET /library/metadata/14452 HTTP/1.1"));
    }

    #[test]
    fn playlist_uri_for_metadata_rejects_unplayable_metadata() {
        let metadata = PlexMediaMetadata {
            rating_key: "14452".to_owned(),
            title: "Episode 11".to_owned(),
            media_type: PlexMediaType::Episode,
            duration_millis: Some(1470058),
            parts: Vec::new(),
        };

        let error = playlist_uri_for_metadata("machine", &metadata, None)
            .expect_err("metadata without parts should fail")
            .to_string();

        assert!(error.contains("playable part"));
    }

    #[test]
    fn file_path_matching_accepts_mapped_drive_suffix() {
        let file = LocalFileUpdate::new(
            "[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv",
        )
        .with_path(
            "E:/Anime/Isekai Nonbiri Nouka 2/[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv",
        );
        let results = vec![PlexMediaSearchResult {
            rating_key: "episode-5".to_owned(),
            title: "Episode 5".to_owned(),
            parent_title: None,
            grandparent_title: None,
            media_type: PlexMediaType::Episode,
            duration_millis: None,
            file_paths: vec![
                "\\\\RAPTOR\\Media\\Anime\\Isekai Nonbiri Nouka 2\\[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv"
                    .to_owned(),
            ],
        }];

        let matched =
            choose_file_path_media_match(&file, &results).expect("path match should be unique");

        assert_eq!(matched.rating_key, "episode-5");
    }

    #[test]
    fn file_path_matching_accepts_plex_library_root_suffix() {
        let file = LocalFileUpdate::new(
            "[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv",
        )
        .with_path(
            "E:/Anime/Isekai Nonbiri Nouka 2/[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv",
        );
        let results = vec![PlexMediaSearchResult {
            rating_key: "12706".to_owned(),
            title: "Another Peaceful Day".to_owned(),
            parent_title: None,
            grandparent_title: None,
            media_type: PlexMediaType::Episode,
            duration_millis: None,
            file_paths: vec![
                "/tv/Isekai Nonbiri Nouka 2/[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv"
                    .to_owned(),
            ],
        }];

        let matched = choose_file_path_media_match(&file, &results)
            .expect("library root suffix should match");

        assert_eq!(matched.rating_key, "12706");
    }

    #[test]
    fn sync_engine_prefers_file_path_match_before_title_search() {
        let transport = FakeTransport::default();
        transport
            .file_search_results
            .borrow_mut()
            .push(PlexMediaSearchResult {
                rating_key: "episode-5".to_owned(),
                title: "Episode 5".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Episode,
                duration_millis: None,
                file_paths: vec![
                    "\\\\RAPTOR\\Media\\Anime\\Isekai Nonbiri Nouka 2\\[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv"
                        .to_owned(),
                ],
            });
        let mut engine = configured_engine(transport.clone());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let event = PlexWatchEvent::new(
            LocalFileUpdate::new(
                "[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv",
            )
            .with_path(
                "E:/Anime/Isekai Nonbiri Nouka 2/[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv",
            ),
        )
        .with_position_seconds(1.0)
        .with_paused(false)
        .with_changed_at(now);

        let status = engine.tick(Some(event), now);

        assert_eq!(
            transport.file_searches.borrow().as_slice(),
            &["[SubsPlease] Isekai Nonbiri Nouka S2 - 05 (1080p) [6706CE18].mkv"]
        );
        assert!(transport.searches.borrow().is_empty());
        assert_eq!(
            status
                .current_item
                .as_ref()
                .map(|item| item.rating_key.as_str()),
            Some("episode-5")
        );
        assert_eq!(status.last_error, None);
    }

    #[test]
    fn sync_engine_retries_missing_match_after_interval() {
        let transport = FakeTransport::default();
        let mut engine = configured_engine(transport.clone());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let event = PlexWatchEvent::new(movie_file())
            .with_position_seconds(1.0)
            .with_paused(false)
            .with_changed_at(now);

        engine.tick(Some(event.clone()), now);
        engine.tick(
            Some(event.clone()),
            now + MATCH_RETRY_INTERVAL
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_default(),
        );
        transport
            .file_search_results
            .borrow_mut()
            .push(PlexMediaSearchResult {
                rating_key: "123".to_owned(),
                title: "Example Movie 2024".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7_200_000),
                file_paths: vec!["C:/Media/Example.Movie.2024.mkv".to_owned()],
            });
        let status = engine.tick(
            Some(event),
            now + MATCH_RETRY_INTERVAL + Duration::from_secs(1),
        );

        assert_eq!(
            transport.file_searches.borrow().as_slice(),
            &["Example.Movie.2024.mkv", "Example.Movie.2024.mkv",]
        );
        assert_eq!(
            transport.searches.borrow().as_slice(),
            &["example movie 2024"]
        );
        assert_eq!(
            status
                .current_item
                .as_ref()
                .map(|item| item.rating_key.as_str()),
            Some("123")
        );
        assert_eq!(status.last_error, None);
    }

    #[test]
    fn matching_prefers_exact_title_and_duration() {
        let file = movie_file();
        let results = vec![
            PlexMediaSearchResult {
                rating_key: "wrong".to_owned(),
                title: "Example Movie".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Movie,
                duration_millis: Some(3_600_000),
                file_paths: Vec::new(),
            },
            PlexMediaSearchResult {
                rating_key: "right".to_owned(),
                title: "Example Movie 2024".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7_200_000),
                file_paths: Vec::new(),
            },
        ];

        let matched = choose_best_media_match(&file, &results).expect("match should be unique");

        assert_eq!(matched.rating_key, "right");
    }

    #[test]
    fn matching_rejects_ambiguous_results() {
        let file = LocalFileUpdate::new("Pilot.mkv").with_duration_seconds(1800.0);
        let results = vec![
            PlexMediaSearchResult {
                rating_key: "1".to_owned(),
                title: "Pilot".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Episode,
                duration_millis: Some(1_800_000),
                file_paths: Vec::new(),
            },
            PlexMediaSearchResult {
                rating_key: "2".to_owned(),
                title: "Pilot".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Episode,
                duration_millis: Some(1_800_000),
                file_paths: Vec::new(),
            },
        ];

        assert!(choose_best_media_match(&file, &results).is_none());
    }

    #[test]
    fn cache_keys_prefer_normalized_paths() {
        let file = movie_file();

        assert_eq!(
            cache_key_for_file(&file).as_deref(),
            Some("path:c:/media/example.movie.2024.mkv")
        );
    }

    #[test]
    fn server_scoped_cache_keys_separate_same_file_across_servers() {
        let file = movie_file();
        let first_config = PlexClientConfig {
            selected_server_id: Some("Raptor Machine".to_owned()),
            selected_server_url: Some("http://plex-a.local:32400".to_owned()),
            ..PlexClientConfig::default()
        };
        let second_config = PlexClientConfig {
            selected_server_id: Some("Tower Machine".to_owned()),
            selected_server_url: Some("http://plex-b.local:32400".to_owned()),
            ..PlexClientConfig::default()
        };

        assert_ne!(
            server_scoped_cache_key_for_file(&first_config, &file),
            server_scoped_cache_key_for_file(&second_config, &file)
        );
    }

    #[test]
    fn timeline_report_uses_milliseconds_and_state() {
        let event = PlexWatchEvent::new(movie_file())
            .with_position_seconds(12.345)
            .with_duration_seconds(7200.0)
            .with_paused(true);
        let item = PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Example Movie".to_owned(),
            media_type: PlexMediaType::Movie,
            duration_millis: None,
        };

        let report = timeline_report_for_event(&event, &item);

        assert_eq!(report.rating_key, "123");
        assert_eq!(report.state, PlexTimelineState::Paused);
        assert_eq!(report.time_millis, 12_345);
        assert_eq!(report.duration_millis, Some(7_200_000));
    }

    #[test]
    fn sync_engine_searches_once_caches_and_throttles_reports() {
        let transport = FakeTransport::default();
        transport
            .search_results
            .borrow_mut()
            .push(PlexMediaSearchResult {
                rating_key: "123".to_owned(),
                title: "Example Movie 2024".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7_200_000),
                file_paths: Vec::new(),
            });
        let mut engine = configured_engine(transport.clone());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let event = PlexWatchEvent::new(movie_file())
            .with_position_seconds(1.0)
            .with_paused(false)
            .with_changed_at(now);

        engine.tick(Some(event.clone()), now);
        engine.tick(
            Some(event.with_position_seconds(2.0)),
            now + Duration::from_secs(2),
        );

        assert_eq!(
            transport.searches.borrow().as_slice(),
            &["example movie 2024"]
        );
        let reports = transport.reports.borrow();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].rating_key, "123");
        assert_eq!(reports[0].state, PlexTimelineState::Playing);
    }

    #[test]
    fn sync_engine_reports_after_interval_and_stops_on_file_clear() {
        let transport = FakeTransport::default();
        transport
            .search_results
            .borrow_mut()
            .push(PlexMediaSearchResult {
                rating_key: "123".to_owned(),
                title: "Example Movie 2024".to_owned(),
                parent_title: None,
                grandparent_title: None,
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7_200_000),
                file_paths: Vec::new(),
            });
        let mut engine = configured_engine(transport.clone());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);

        engine.tick(
            Some(
                PlexWatchEvent::new(movie_file())
                    .with_position_seconds(1.0)
                    .with_paused(false),
            ),
            now,
        );
        engine.tick(
            Some(
                PlexWatchEvent::new(movie_file())
                    .with_position_seconds(12.0)
                    .with_paused(false),
            ),
            now + Duration::from_secs(11),
        );
        engine.tick(None, now + Duration::from_secs(12));

        let reports = transport.reports.borrow();
        assert_eq!(reports.len(), 3);
        assert_eq!(reports[1].time_millis, 12_000);
        assert_eq!(reports[2].state, PlexTimelineState::Stopped);
    }
}
