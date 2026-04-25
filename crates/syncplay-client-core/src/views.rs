use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientUserView {
    pub room: Option<String>,
    pub ready: Option<bool>,
    pub has_file: bool,
    pub file_name: Option<String>,
    pub file_size: Option<Value>,
    pub file_duration: Option<Value>,
    pub features: Option<Value>,
    pub controller: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomPlaylistView {
    pub files: Vec<String>,
    pub index: Option<i64>,
    pub set_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoomPlaystateView {
    pub position: Option<f64>,
    pub paused: Option<bool>,
    pub do_seek: Option<bool>,
    pub set_by: Option<String>,
}
