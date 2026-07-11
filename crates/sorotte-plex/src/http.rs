//! Blocking Plex HTTP transport. Application code should prefer responsibility-specific services.

pub use crate::{
    PlexHttpClient, PlexMetadataTransport, PlexServerDiscoveryTransport, PlexSyncTransport,
};
