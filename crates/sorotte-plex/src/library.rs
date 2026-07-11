//! Plex library search and metadata boundary.

use crate::{PlexClientConfig, PlexHttpClient, PlexResult, SecretPlexPlaybackUrl};
pub use crate::{
    PlexMatchedItem, PlexMediaMetadata, PlexMediaSearchResult, PlexMediaType, PlexPlayablePart,
    PlexPlaylistUri, PlexStreamTarget,
};

#[derive(Debug, Clone, Copy)]
pub struct PlexLibraryService<'a> {
    http: &'a PlexHttpClient,
}

impl<'a> PlexLibraryService<'a> {
    pub const fn new(http: &'a PlexHttpClient) -> Self {
        Self { http }
    }

    pub fn search_file_name(
        &self,
        server_url: &str,
        token: &str,
        file_name: &str,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        self.http
            .search_media_by_file_name(server_url, token, file_name)
    }

    pub fn search_selected(
        &self,
        config: &PlexClientConfig,
        query: &str,
        limit: usize,
    ) -> PlexResult<Vec<PlexMediaSearchResult>> {
        self.http.search_selected_server_media(config, query, limit)
    }

    pub fn playlist_uri(
        &self,
        config: &PlexClientConfig,
        rating_key: &str,
    ) -> PlexResult<PlexPlaylistUri> {
        self.http
            .playlist_uri_for_selected_server_rating_key(config, rating_key)
    }

    pub fn metadata(
        &self,
        server_url: &str,
        token: &str,
        rating_key: &str,
    ) -> PlexResult<PlexMediaMetadata> {
        self.http
            .metadata_by_rating_key(server_url, token, rating_key)
    }

    pub fn stream_url(
        &self,
        server_url: &str,
        token: &str,
        part: &PlexPlayablePart,
    ) -> PlexResult<SecretPlexPlaybackUrl> {
        self.http.build_part_stream_url(server_url, token, part)
    }
}
