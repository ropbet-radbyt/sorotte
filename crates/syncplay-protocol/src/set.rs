use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SetPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FilePayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<BTreeMap<String, UserSetPayload>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "controllerAuth"
    )]
    pub controller_auth: Option<ControllerAuthPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "newControlledRoom"
    )]
    pub new_controlled_room: Option<NewControlledRoomPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<ReadyPayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "playlistChange"
    )]
    pub playlist_change: Option<PlaylistChangePayload>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "playlistIndex"
    )]
    pub playlist_index: Option<PlaylistIndexPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl SetPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: RoomRef) -> Self {
        self.room = Some(room);
        self
    }

    pub fn with_file(mut self, file: FilePayload) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_user(mut self, user: BTreeMap<String, UserSetPayload>) -> Self {
        self.user = Some(user);
        self
    }

    pub fn with_controller_auth(mut self, controller_auth: ControllerAuthPayload) -> Self {
        self.controller_auth = Some(controller_auth);
        self
    }

    pub fn with_new_controlled_room(
        mut self,
        new_controlled_room: NewControlledRoomPayload,
    ) -> Self {
        self.new_controlled_room = Some(new_controlled_room);
        self
    }

    pub fn with_ready(mut self, ready: ReadyPayload) -> Self {
        self.ready = Some(ready);
        self
    }

    pub fn with_playlist_change(mut self, playlist_change: PlaylistChangePayload) -> Self {
        self.playlist_change = Some(playlist_change);
        self
    }

    pub fn with_playlist_index(mut self, playlist_index: PlaylistIndexPayload) -> Self {
        self.playlist_index = Some(playlist_index);
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FilePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl FilePayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_duration(mut self, duration: f64) -> Self {
        self.duration = Some(duration);
        self
    }

    pub fn with_size(mut self, size: Value) -> Self {
        self.size = Some(size);
        self
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UserSetPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "isReady")]
    pub is_ready: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl UserSetPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: RoomRef) -> Self {
        self.room = Some(room);
        self
    }

    pub fn with_file(mut self, file: Value) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_event(mut self, event: Value) -> Self {
        self.event = Some(event);
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }

    pub fn with_controller(mut self, controller: bool) -> Self {
        self.controller = Some(controller);
        self
    }

    pub fn with_is_ready(mut self, is_ready: bool) -> Self {
        self.is_ready = Some(is_ready);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ControllerAuthPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ControllerAuthPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: impl Into<String>) -> Self {
        self.room = Some(room.into());
        self
    }

    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = Some(success);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NewControlledRoomPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "roomName")]
    pub room_name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl NewControlledRoomPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn with_room_name(mut self, room_name: impl Into<String>) -> Self {
        self.room_name = Some(room_name.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyPayload {
    #[serde(rename = "isReady")]
    pub is_ready: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "manuallyInitiated"
    )]
    pub manually_initiated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "setBy")]
    pub set_by: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ReadyPayload {
    pub fn new(is_ready: bool) -> Self {
        Self {
            is_ready,
            manually_initiated: None,
            username: None,
            set_by: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_manually_initiated(mut self, manually_initiated: bool) -> Self {
        self.manually_initiated = Some(manually_initiated);
        self
    }

    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    pub fn with_set_by(mut self, set_by: impl Into<String>) -> Self {
        self.set_by = Some(set_by.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistChangePayload {
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PlaylistChangePayload {
    pub fn new(files: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            files: files.into_iter().map(Into::into).collect(),
            user: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistIndexPayload {
    pub index: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl PlaylistIndexPayload {
    pub fn new(index: i64) -> Self {
        Self {
            index,
            user: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }
}
