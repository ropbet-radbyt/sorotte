//! Plex server discovery boundary.

use crate::{PlexHttpClient, PlexResult};
pub use crate::{PlexServerConnection, PlexServerConnectionKind, PlexServerDiscoveryTransport};

#[derive(Debug, Clone, Copy)]
pub struct PlexDiscoveryService<'a> {
    http: &'a PlexHttpClient,
}

impl<'a> PlexDiscoveryService<'a> {
    pub const fn new(http: &'a PlexHttpClient) -> Self {
        Self { http }
    }

    pub fn discover(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>> {
        self.http.discover_servers(user_token)
    }

    pub fn verify(&self, server: &PlexServerConnection) -> PlexResult<()> {
        self.http.verify_server_connection(server)
    }

    pub fn machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String> {
        self.http.server_machine_identifier(server_url, token)
    }
}
