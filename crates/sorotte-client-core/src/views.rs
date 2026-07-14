use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientUserView {
    pub room: Option<String>,
    pub ready: Option<bool>,
    pub file: Option<SharedFile>,
    pub capabilities: Option<PeerCapabilities>,
    pub controller: bool,
}

#[derive(Clone, PartialEq, Default)]
pub struct ClientMediaMatchPeerFileState {
    pub username: String,
    pub has_file: bool,
    pub file_name: Option<String>,
    pub file_size: Option<FileSize>,
    pub file_duration: Option<f64>,
    pub media_match_signature: Option<MediaMatchWireSignature>,
}

impl std::fmt::Debug for ClientMediaMatchPeerFileState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClientMediaMatchPeerFileState")
            .field("username", &self.username)
            .field("has_file", &self.has_file)
            .field(
                "file_name",
                &self
                    .file_name
                    .as_ref()
                    .map(|_| sorotte_secret::REDACTED_SECRET),
            )
            .field("file_size", &self.file_size)
            .field("file_duration", &self.file_duration)
            .field("media_match_signature", &self.media_match_signature)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct RoomPlaylistView {
    pub files: Vec<String>,
    pub index: Option<i64>,
    pub set_by: Option<String>,
    pub revision: u64,
}

impl std::fmt::Debug for RoomPlaylistView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RoomPlaylistView")
            .field("files_count", &self.files.len())
            .field("index", &self.index)
            .field("set_by", &self.set_by)
            .field("revision", &self.revision)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoomPlaystateView {
    pub position: Option<f64>,
    pub paused: Option<bool>,
    pub do_seek: Option<bool>,
    pub set_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomPlaystateAuthority {
    LegacyRemoteUser,
    LegacyLocalEcho,
    ServerBarrier {
        media_generation: u64,
        state_revision: Option<u64>,
    },
    ServerBufferingPolicy {
        media_generation: u64,
    },
}
