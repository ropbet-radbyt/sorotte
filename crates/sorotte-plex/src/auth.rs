//! Plex PIN authentication boundary.

pub use crate::{PlexAuthPollResult, PlexAuthSession};
use crate::{PlexHttpClient, PlexResult};

#[derive(Debug, Clone, Copy)]
pub struct PlexAuthService<'a> {
    http: &'a PlexHttpClient,
}

impl<'a> PlexAuthService<'a> {
    pub const fn new(http: &'a PlexHttpClient) -> Self {
        Self { http }
    }

    pub fn start(&self) -> PlexResult<PlexAuthSession> {
        self.http.start_auth()
    }

    pub fn poll(&self, pin_id: u64) -> PlexResult<PlexAuthPollResult> {
        self.http.poll_auth(pin_id)
    }
}
