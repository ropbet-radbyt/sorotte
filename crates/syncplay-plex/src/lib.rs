use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use syncplay_player_api::LocalFileUpdate;

const DEFAULT_PLEX_TV_BASE_URL: &str = "https://plex.tv";
const DEFAULT_PLEX_AUTH_APP_URL: &str = "https://app.plex.tv/auth";
const DEFAULT_CLIENT_PRODUCT: &str = "Syncplay Rust";
const DEFAULT_TIMELINE_INTERVAL: Duration = Duration::from_secs(10);
const SEEK_REPORT_THRESHOLD_MILLIS: i64 = 15_000;
static RUSTLS_PROVIDER_INIT: OnceLock<()> = OnceLock::new();

pub type PlexResult<T> = Result<T, PlexError>;

#[derive(Debug, thiserror::Error)]
pub enum PlexError {
    #[error("failed Plex HTTP request: {0}")]
    Http(#[from] reqwest::Error),
    #[error("failed to parse Plex response: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to read or write Plex cache: {0}")]
    Io(#[from] std::io::Error),
    #[error("Plex returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("Plex server is not configured")]
    MissingServer,
    #[error("Plex token is not configured")]
    MissingToken,
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
    pub auth_token: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlexClientConfig {
    pub enabled: bool,
    pub user_token: Option<String>,
    pub selected_server_id: Option<String>,
    pub selected_server_url: Option<String>,
    pub selected_server_token: Option<String>,
}

impl PlexClientConfig {
    pub fn selected_server_token_or_user_token(&self) -> Option<&str> {
        self.selected_server_token
            .as_deref()
            .or(self.user_token.as_deref())
            .filter(|token| !token.trim().is_empty())
    }

    pub fn has_selected_server(&self) -> bool {
        self.selected_server_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
            && self.selected_server_token_or_user_token().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlexServerConnectionKind {
    Local,
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

impl Default for PlexServerConnectionKind {
    fn default() -> Self {
        Self::Remote
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlexServerConnection {
    pub name: String,
    pub machine_identifier: String,
    pub uri: String,
    pub access_token: String,
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
    pub media_type: PlexMediaType,
    pub duration_millis: Option<u64>,
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
            .user_agent(format!("syncplay-rs-plex/{}", env!("CARGO_PKG_VERSION")))
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
                .map(ToOwned::to_owned),
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
        if server.access_token.trim().is_empty() {
            return Err(PlexError::MissingToken);
        }
        let response = self
            .client
            .get(server.uri.trim_end_matches('/'))
            .headers(self.plex_headers(Some(&server.access_token)))
            .send()?;
        if !response.status().is_success() {
            return Err(PlexError::InvalidResponse(format!(
                "server verification returned HTTP {}",
                response.status()
            )));
        }
        Ok(())
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
        if let Some(token) = token {
            insert_header_value(&mut headers, HeaderName::from_static("x-plex-token"), token);
        }
        headers
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
            ("ratingKey", report.rating_key.as_str()),
            ("state", report.state.as_plex_value()),
        ];
        let time = report.time_millis.to_string();
        let duration = report.duration_millis.unwrap_or(0).to_string();
        query.push(("time", &time));
        query.push(("duration", &duration));

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

pub trait PlexSyncTransport {
    fn search_media(
        &self,
        server_url: &str,
        token: &str,
        query: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>>;

    fn report_timeline(
        &self,
        server_url: &str,
        token: &str,
        report: &PlexTimelineReport,
    ) -> PlexResult<()>;
}

#[derive(Debug, Clone)]
pub struct PlexSyncEngine<T> {
    config: PlexClientConfig,
    transport: T,
    cache: PlexMatchCache,
    status: PlexSyncStatus,
    current_file_key: Option<String>,
    last_report_signature: Option<ReportSignature>,
    unmatched_keys: BTreeSet<String>,
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
            unmatched_keys: BTreeSet::new(),
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
        let Some(file_key) = cache_key_for_file(&event.file) else {
            self.status = PlexSyncStatus::ready();
            return Ok(());
        };

        if self.current_file_key.as_deref() != Some(file_key.as_str()) {
            self.report_stop_if_needed(&server_url, &token, now)?;
            self.current_file_key = Some(file_key.clone());
            self.last_report_signature = None;
            self.status.current_item = None;
        }

        let Some(item) = self.resolve_match(&server_url, &token, &event, &file_key)? else {
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
    ) -> PlexResult<Option<PlexMatchedItem>> {
        if let Some(cached) = self.cache.entries.get(file_key).cloned() {
            let item: PlexMatchedItem = cached.into();
            self.status.current_item = Some(item.clone());
            return Ok(Some(item));
        }
        if self.unmatched_keys.contains(file_key) {
            return Ok(None);
        }
        let query = media_search_query_for_file(&event.file);
        if query.is_empty() {
            self.unmatched_keys.insert(file_key.to_owned());
            return Ok(None);
        }
        let results = self.transport.search_media(server_url, token, &query)?;
        let matched = choose_best_media_match(&event.file, &results);
        match matched {
            Some(item) => {
                self.cache
                    .entries
                    .insert(file_key.to_owned(), PlexCachedMatch::from(item.clone()));
                self.status.current_item = Some(item.clone());
                Ok(Some(item))
            }
            None => {
                self.unmatched_keys.insert(file_key.to_owned());
                Ok(None)
            }
        }
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
    let Some((best_score, best)) = scored.first() else {
        return None;
    };
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
                        access_token: access_token.clone(),
                        owned,
                        has_local_connection,
                        connection_kind,
                    })
                })
                .min_by_key(|server| server_connection_rank(server))
        })
        .collect()
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
    let Some(authority) = uri_authority(uri) else {
        return false;
    };
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority)
        .trim_matches(['[', ']']);
    let host_without_port = host.rsplit_once(':').map(|(host, _)| host).unwrap_or(host);
    host_without_port == "localhost"
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

fn parse_search_response(json: &Value) -> Vec<PlexMediaSearchResult> {
    let mut output = Vec::new();
    collect_search_results(json, &mut output);
    output
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
                output.push(PlexMediaSearchResult {
                    rating_key,
                    title,
                    media_type: PlexMediaType::from_plex_type(type_name),
                    duration_millis: map.get("duration").and_then(value_as_u64),
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

fn insert_header_value(
    headers: &mut reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
    value: &str,
) {
    if let Ok(value) = reqwest::header::HeaderValue::from_str(value) {
        headers.insert(name, value);
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

    #[derive(Clone, Default)]
    struct FakeTransport {
        searches: Rc<RefCell<Vec<String>>>,
        search_results: Rc<RefCell<Vec<PlexMediaSearchResult>>>,
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

    fn configured_engine(transport: FakeTransport) -> PlexSyncEngine<FakeTransport> {
        PlexSyncEngine::new(
            PlexClientConfig {
                enabled: true,
                selected_server_url: Some("http://plex.local:32400".to_owned()),
                selected_server_token: Some("server-token".to_owned()),
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
        assert_eq!(servers[0].access_token, "server-token");
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
        assert_eq!(servers[0].access_token, "shared-token");
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
        assert_eq!(servers[0].access_token, "raptor-token");
        assert!(servers[0].owned);
        assert!(!servers[0].has_local_connection);
        assert_eq!(servers[0].connection_kind, PlexServerConnectionKind::Remote);
        assert_eq!(servers[1].name, "zzzzzzzzzzzzzzzzzzzzzz");
        assert_eq!(servers[1].access_token, "shared-token");
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
            access_token: "raptor-token".to_owned(),
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
                    access_token: "raptor-token".to_owned(),
                    owned: true,
                    has_local_connection: false,
                    connection_kind: PlexServerConnectionKind::Relay,
                },
                PlexServerConnection {
                    name: "Raptor".to_owned(),
                    machine_identifier: "raptor-machine".to_owned(),
                    uri: "https://125-209-152-187.raptor-machine.plex.direct:32400".to_owned(),
                    access_token: "raptor-token".to_owned(),
                    owned: true,
                    has_local_connection: false,
                    connection_kind: PlexServerConnectionKind::Remote,
                },
                PlexServerConnection {
                    name: "Tower".to_owned(),
                    machine_identifier: "tower-machine".to_owned(),
                    uri: "https://180-181-237-20.tower-machine.plex.direct:32400".to_owned(),
                    access_token: "tower-token".to_owned(),
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
            access_token: "server-token".to_owned(),
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
    fn search_response_collects_video_metadata() {
        let json: Value = serde_json::json!({
            "MediaContainer": {
                "Metadata": [
                    { "ratingKey": "1", "type": "movie", "title": "Movie", "duration": 60000 },
                    { "ratingKey": "2", "type": "track", "title": "Song", "duration": 1000 }
                ]
            }
        });

        let results = parse_search_response(&json);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].rating_key, "1");
        assert_eq!(results[0].media_type, PlexMediaType::Movie);
    }

    #[test]
    fn matching_prefers_exact_title_and_duration() {
        let file = movie_file();
        let results = vec![
            PlexMediaSearchResult {
                rating_key: "wrong".to_owned(),
                title: "Example Movie".to_owned(),
                media_type: PlexMediaType::Movie,
                duration_millis: Some(3600_000),
            },
            PlexMediaSearchResult {
                rating_key: "right".to_owned(),
                title: "Example Movie 2024".to_owned(),
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7200_000),
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
                media_type: PlexMediaType::Episode,
                duration_millis: Some(1800_000),
            },
            PlexMediaSearchResult {
                rating_key: "2".to_owned(),
                title: "Pilot".to_owned(),
                media_type: PlexMediaType::Episode,
                duration_millis: Some(1800_000),
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
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7_200_000),
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
                media_type: PlexMediaType::Movie,
                duration_millis: Some(7_200_000),
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
