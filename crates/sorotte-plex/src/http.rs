//! Blocking Plex HTTP transport. Application code should prefer responsibility-specific services.

pub use crate::{
    PlexHttpClient, PlexMetadataTransport, PlexServerDiscoveryTransport, PlexSyncTransport,
};

use std::{
    io::Read,
    time::{Duration, Instant},
};

use reqwest::{
    Url,
    blocking::{RequestBuilder, Response},
};
use serde_json::Value;

use crate::{PlexError, PlexResult};

pub(crate) const METADATA_LIMIT: usize = 8 * 1024 * 1024;
const SEARCH_BYTE_LIMIT: usize = 32 * 1024 * 1024;
const SEARCH_REQUEST_LIMIT: usize = 64;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REDIRECT_LIMIT: usize = 10;

pub(crate) fn same_origin(source: &Url, target: &Url) -> bool {
    matches!(source.scheme(), "http" | "https")
        && source.scheme() == target.scheme()
        && source.host_str() == target.host_str()
        && source.port_or_known_default() == target.port_or_known_default()
        && target.username().is_empty()
        && target.password().is_none()
}

pub(crate) fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if !attempt
            .previous()
            .first()
            .is_some_and(|source| same_origin(source, attempt.url()))
        {
            // Never include the redirect URL: its query may itself contain credentials.
            attempt.error("Plex redirect left the original HTTP origin")
        } else if attempt.previous().len() >= REDIRECT_LIMIT {
            attempt.error("Plex redirect limit exceeded")
        } else {
            attempt.follow()
        }
    })
}

pub(crate) fn read_json(response: Response, label: &str) -> PlexResult<Value> {
    read_json_limited(
        response,
        label,
        METADATA_LIMIT,
        Some(Instant::now() + REQUEST_TIMEOUT),
    )
    .map(|(json, _)| json)
}

fn read_json_limited(
    mut response: Response,
    label: &str,
    limit: usize,
    deadline: Option<Instant>,
) -> PlexResult<(Value, usize)> {
    let status = response.status();
    if !status.is_success() {
        // Error bodies are irrelevant and can be arbitrarily large or never complete.
        return Err(PlexError::InvalidResponse(format!(
            "{label} returned HTTP {status}"
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(body_limit_error(label));
    }
    let mut body = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(PlexError::Http("Plex search deadline exceeded".to_owned()));
        }
        let available = buffer.len().min(limit.saturating_sub(body.len()) + 1);
        let count = response
            .read(&mut buffer[..available])
            .map_err(|_| PlexError::Http(format!("failed to read {label} response")))?;
        if count == 0 {
            break;
        }
        if count > limit.saturating_sub(body.len()) {
            return Err(body_limit_error(label));
        }
        body.extend_from_slice(&buffer[..count]);
    }
    Ok((serde_json::from_slice(&body)?, body.len()))
}

fn body_limit_error(label: &str) -> PlexError {
    PlexError::InvalidResponse(format!("{label} response exceeded its byte budget"))
}

/// One budget spans the section inventory and all fallback queries of a user search.
pub(crate) struct SearchContext<'a> {
    pub server_url: &'a str,
    pub token: &'a str,
    remaining_bytes: usize,
    remaining_requests: usize,
    deadline: Instant,
}

impl<'a> SearchContext<'a> {
    pub fn new(server_url: &'a str, token: &'a str) -> Self {
        Self {
            server_url,
            token,
            remaining_bytes: SEARCH_BYTE_LIMIT,
            remaining_requests: SEARCH_REQUEST_LIMIT,
            deadline: Instant::now() + SEARCH_TIMEOUT,
        }
    }

    pub fn send_json(&mut self, request: RequestBuilder, label: &str) -> PlexResult<Value> {
        let remaining_time = self.deadline.saturating_duration_since(Instant::now());
        if self.remaining_requests == 0 || self.remaining_bytes == 0 || remaining_time.is_zero() {
            return Err(PlexError::InvalidResponse(
                "Plex search budget exhausted".to_owned(),
            ));
        }
        self.remaining_requests -= 1;
        let response = request
            .timeout(remaining_time.min(REQUEST_TIMEOUT))
            .send()?;
        let (json, bytes) = read_json_limited(
            response,
            label,
            self.remaining_bytes.min(METADATA_LIMIT),
            Some(self.deadline),
        )?;
        self.remaining_bytes -= bytes;
        Ok(json)
    }
}
