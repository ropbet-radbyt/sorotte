use super::*;
use crate::redacted_debug::{
    RedactedJsonMap, RedactedOptionalJsonValue, RedactedOptionalSensitiveText,
    RedactedOptionalText, RedactedTextList,
};

#[derive(Clone, Serialize, Deserialize, Default)]
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
    #[serde(skip)]
    pub command_order: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for SetPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SetPayload")
            .field("room", &self.room)
            .field("file", &self.file)
            .field("user", &self.user)
            .field("controller_auth", &self.controller_auth)
            .field("new_controlled_room", &self.new_controlled_room)
            .field("ready", &self.ready)
            .field("playlist_change", &self.playlist_change)
            .field("playlist_index", &self.playlist_index)
            .field(
                "features",
                &RedactedOptionalJsonValue(self.features.as_ref()),
            )
            .field("command_order", &self.command_order)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl PartialEq for SetPayload {
    fn eq(&self, other: &Self) -> bool {
        self.room == other.room
            && self.file == other.file
            && self.user == other.user
            && self.controller_auth == other.controller_auth
            && self.new_controlled_room == other.new_controlled_room
            && self.ready == other.ready
            && self.playlist_change == other.playlist_change
            && self.playlist_index == other.playlist_index
            && self.features == other.features
            && self.extra == other.extra
    }
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

    pub fn with_playback_barrier_v1(mut self, extension: PlaybackBarrierSetExtension) -> Self {
        playback_barrier::insert_extension(&mut self.extra, &extension);
        self
    }

    pub fn playback_barrier_v1(&self) -> serde_json::Result<Option<PlaybackBarrierSetExtension>> {
        playback_barrier::decode_extension(&self.extra)
    }

    pub fn with_readiness_v2(mut self, extension: ReadinessSetExtension) -> Self {
        readiness::insert_extension(&mut self.extra, &extension);
        self
    }

    pub fn readiness_v2(&self) -> serde_json::Result<Option<ReadinessSetExtension>> {
        readiness::decode_extension(&self.extra)
    }

    pub fn with_command_order(mut self, command_order: Vec<String>) -> Self {
        self.command_order = command_order;
        self
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
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

impl std::fmt::Debug for FilePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FilePayload")
            .field("name", &RedactedOptionalSensitiveText(self.name.as_deref()))
            .field("duration", &self.duration)
            .field("size", &RedactedOptionalJsonValue(self.size.as_ref()))
            .field("path", &RedactedOptionalText(self.path.as_deref()))
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
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

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
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

impl std::fmt::Debug for UserSetPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserSetPayload")
            .field("room", &self.room)
            .field("file", &RedactedOptionalJsonValue(self.file.as_ref()))
            .field("event", &RedactedOptionalJsonValue(self.event.as_ref()))
            .field(
                "features",
                &RedactedOptionalJsonValue(self.features.as_ref()),
            )
            .field("controller", &self.controller)
            .field("is_ready", &self.is_ready)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
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

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ControllerAuthPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_secret",
        deserialize_with = "deserialize_optional_secret"
    )]
    pub password: Option<SecretValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for ControllerAuthPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControllerAuthPayload")
            .field("room", &self.room)
            .field("password", &self.password)
            .field("user", &self.user)
            .field("success", &self.success)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl ControllerAuthPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_room(mut self, room: impl Into<String>) -> Self {
        self.room = Some(room.into());
        self
    }

    pub fn with_password(mut self, password: impl Into<SecretValue>) -> Self {
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

#[derive(Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct NewControlledRoomPayload {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_optional_secret",
        deserialize_with = "deserialize_optional_secret"
    )]
    pub password: Option<SecretValue>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "roomName")]
    pub room_name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for NewControlledRoomPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NewControlledRoomPayload")
            .field("password", &self.password)
            .field("room_name", &self.room_name)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl NewControlledRoomPayload {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_password(mut self, password: impl Into<SecretValue>) -> Self {
        self.password = Some(password.into());
        self
    }

    pub fn with_room_name(mut self, room_name: impl Into<String>) -> Self {
        self.room_name = Some(room_name.into());
        self
    }
}

fn serialize_optional_secret<S>(
    value: &Option<SecretValue>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    value
        .as_ref()
        .map(SecretValue::expose_secret)
        .serialize(serializer)
}

fn deserialize_optional_secret<'de, D>(deserializer: D) -> Result<Option<SecretValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(|value| value.map(SecretValue::from))
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyPayload {
    #[serde(default, rename = "isReady")]
    pub is_ready: Option<bool>,
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

impl std::fmt::Debug for ReadyPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadyPayload")
            .field("is_ready", &self.is_ready)
            .field("manually_initiated", &self.manually_initiated)
            .field("username", &self.username)
            .field("set_by", &self.set_by)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl ReadyPayload {
    pub fn new(is_ready: impl Into<Option<bool>>) -> Self {
        Self {
            is_ready: is_ready.into(),
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

#[derive(Clone, PartialEq)]
pub struct PlaylistChangePayload {
    pub files: Vec<String>,
    pub user: Option<String>,
    pub user_is_null: bool,
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for PlaylistChangePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaylistChangePayload")
            .field("files", &RedactedTextList(&self.files))
            .field("user", &self.user)
            .field("user_is_null", &self.user_is_null)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl PlaylistChangePayload {
    pub fn new(files: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            files: files.into_iter().map(Into::into).collect(),
            user: None,
            user_is_null: false,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.user_is_null = false;
        self
    }

    pub fn with_null_user(mut self) -> Self {
        self.user = None;
        self.user_is_null = true;
        self
    }
}

pub const SOROTTE_PLEX_PLAYLIST_URIS_FEATURE: &str = "sorottePlexPlaylistUris";
pub const SOROTTE_PLEX_PLAYLIST_URIS_KEY: &str = SOROTTE_PLEX_PLAYLIST_URIS_FEATURE;
/// Monotonic, server-issued generation for the canonical playlist contents or
/// selection. It is carried on both playlistChange and playlistIndex fanout.
pub const SOROTTE_PLAYLIST_EPOCH_KEY: &str = "sorottePlaylistEpoch";
/// Natural-EOF compare-and-set guard: the selection must still be this index.
pub const SOROTTE_EXPECTED_PLAYLIST_INDEX_KEY: &str = "sorotteExpectedPlaylistIndex";
/// Natural-EOF compare-and-set guard: the canonical playlist generation must
/// still match the generation observed when the player reached EOF.
pub const SOROTTE_EXPECTED_PLAYLIST_EPOCH_KEY: &str = "sorotteExpectedPlaylistEpoch";

impl PlaylistChangePayload {
    pub fn playlist_epoch(&self) -> Option<u64> {
        self.extra
            .get(SOROTTE_PLAYLIST_EPOCH_KEY)
            .and_then(Value::as_u64)
    }

    pub fn with_playlist_epoch(mut self, epoch: u64) -> Self {
        self.extra
            .insert(SOROTTE_PLAYLIST_EPOCH_KEY.to_owned(), Value::from(epoch));
        self
    }
}

pub fn is_sorotte_plex_playlist_uri(value: &str) -> bool {
    value
        .as_bytes()
        .get(.."plex://".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"plex://"))
}

pub fn syncplay_playlist_file_name(entry: &str) -> String {
    plex_playlist_display_name(entry).unwrap_or_else(|| entry.to_owned())
}

pub fn canonical_playlist_files_from_change(payload: &PlaylistChangePayload) -> Vec<String> {
    let Some(sidecar_uris) = payload
        .extra
        .get(SOROTTE_PLEX_PLAYLIST_URIS_KEY)
        .and_then(Value::as_array)
    else {
        return payload.files.clone();
    };

    payload
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            sidecar_uris
                .get(index)
                .and_then(Value::as_str)
                .filter(|uri| is_sorotte_plex_playlist_uri(uri))
                .map(str::to_owned)
                .unwrap_or_else(|| file.clone())
        })
        .collect()
}

pub fn playlist_change_with_plex_sidecar(
    files: impl IntoIterator<Item = impl Into<String>>,
    include_plex_sidecar: bool,
) -> PlaylistChangePayload {
    let canonical_files: Vec<String> = files.into_iter().map(Into::into).collect();
    let (syncplay_files, plex_sidecar) = split_playlist_files_for_syncplay(&canonical_files);
    let mut payload = PlaylistChangePayload::new(syncplay_files);
    if include_plex_sidecar && let Some(plex_sidecar) = plex_sidecar {
        payload
            .extra
            .insert(SOROTTE_PLEX_PLAYLIST_URIS_KEY.to_owned(), plex_sidecar);
    }
    payload
}

pub fn split_playlist_files_for_syncplay(files: &[String]) -> (Vec<String>, Option<Value>) {
    let mut syncplay_files = Vec::with_capacity(files.len());
    let mut plex_sidecar = Vec::with_capacity(files.len());
    let mut has_plex_uri = false;

    for file in files {
        if is_sorotte_plex_playlist_uri(file) {
            has_plex_uri = true;
            syncplay_files.push(syncplay_playlist_file_name(file));
            plex_sidecar.push(Value::String(file.clone()));
        } else {
            syncplay_files.push(file.clone());
            plex_sidecar.push(Value::Null);
        }
    }

    (
        syncplay_files,
        has_plex_uri.then_some(Value::Array(plex_sidecar)),
    )
}

fn plex_playlist_display_name(uri: &str) -> Option<String> {
    if !is_sorotte_plex_playlist_uri(uri) {
        return None;
    }
    plex_query_value(uri, "file")
        .map(|file| {
            file.rsplit(['/', '\\'])
                .next()
                .unwrap_or(file.as_str())
                .to_owned()
        })
        .or_else(|| plex_query_value(uri, "title"))
}

fn plex_query_value(uri: &str, key: &str) -> Option<String> {
    let query = uri.split_once('?')?.1;
    query.split('&').find_map(|entry| {
        let (entry_key, entry_value) = entry.split_once('=').unwrap_or((entry, ""));
        (entry_key == key)
            .then(|| percent_decode_query_value(entry_value))
            .filter(|value| !value.is_empty())
    })
}

fn percent_decode_query_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                if let (Some(high), Some(low)) =
                    (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
                {
                    decoded.push((high << 4) | low);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

impl Serialize for PlaylistChangePayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let extra_len = self
            .extra
            .keys()
            .filter(|key| key.as_str() != "files" && key.as_str() != "user")
            .count();
        let mut map = serializer.serialize_map(Some(
            1 + usize::from(self.user.is_some() || self.user_is_null) + extra_len,
        ))?;
        map.serialize_entry("files", &self.files)?;
        if let Some(user) = &self.user {
            map.serialize_entry("user", user)?;
        } else if self.user_is_null {
            map.serialize_entry("user", &Value::Null)?;
        }
        for (key, value) in &self.extra {
            if key == "files" || key == "user" {
                continue;
            }
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for PlaylistChangePayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut entries = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let files_value = entries
            .remove("files")
            .ok_or_else(|| serde::de::Error::missing_field("files"))?;
        let files: Vec<String> =
            serde_json::from_value(files_value).map_err(serde::de::Error::custom)?;
        let mut payload = PlaylistChangePayload::new(files);
        if let Some(user_value) = entries.remove("user") {
            match user_value {
                Value::String(user) => payload.user = Some(user),
                Value::Null => payload.user_is_null = true,
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "playlistChange.user must be a string or null, got {other}"
                    )));
                }
            }
        }
        payload.extra.extend(entries);
        Ok(payload)
    }
}

#[derive(Clone, PartialEq)]
pub struct PlaylistIndexPayload {
    pub index: i64,
    pub index_is_null: bool,
    pub user: Option<String>,
    pub user_is_null: bool,
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for PlaylistIndexPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlaylistIndexPayload")
            .field("index", &self.index)
            .field("index_is_null", &self.index_is_null)
            .field("user", &self.user)
            .field("user_is_null", &self.user_is_null)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl PlaylistIndexPayload {
    pub fn new(index: i64) -> Self {
        Self {
            index,
            index_is_null: false,
            user: None,
            user_is_null: false,
            extra: BTreeMap::new(),
        }
    }

    pub fn null() -> Self {
        Self {
            index: 0,
            index_is_null: true,
            user: None,
            user_is_null: false,
            extra: BTreeMap::new(),
        }
    }

    pub fn from_optional(index: Option<i64>) -> Self {
        index.map_or_else(Self::null, Self::new)
    }

    pub fn index_value(&self) -> Option<i64> {
        (!self.index_is_null).then_some(self.index)
    }

    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self.user_is_null = false;
        self
    }

    pub fn with_null_user(mut self) -> Self {
        self.user = None;
        self.user_is_null = true;
        self
    }

    pub fn playlist_epoch(&self) -> Option<u64> {
        self.extra
            .get(SOROTTE_PLAYLIST_EPOCH_KEY)
            .and_then(Value::as_u64)
    }

    pub fn expected_playlist_index(&self) -> Option<i64> {
        self.extra
            .get(SOROTTE_EXPECTED_PLAYLIST_INDEX_KEY)
            .and_then(Value::as_i64)
    }

    pub fn expected_playlist_epoch(&self) -> Option<u64> {
        self.extra
            .get(SOROTTE_EXPECTED_PLAYLIST_EPOCH_KEY)
            .and_then(Value::as_u64)
    }

    pub fn has_expected_playlist_state(&self) -> bool {
        self.extra.contains_key(SOROTTE_EXPECTED_PLAYLIST_INDEX_KEY)
            || self.extra.contains_key(SOROTTE_EXPECTED_PLAYLIST_EPOCH_KEY)
    }

    pub fn with_playlist_epoch(mut self, epoch: u64) -> Self {
        self.extra
            .insert(SOROTTE_PLAYLIST_EPOCH_KEY.to_owned(), Value::from(epoch));
        self
    }

    pub fn with_expected_playlist_state(mut self, index: i64, epoch: u64) -> Self {
        self.extra.insert(
            SOROTTE_EXPECTED_PLAYLIST_INDEX_KEY.to_owned(),
            Value::from(index),
        );
        self.extra.insert(
            SOROTTE_EXPECTED_PLAYLIST_EPOCH_KEY.to_owned(),
            Value::from(epoch),
        );
        self
    }
}

impl Serialize for PlaylistIndexPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let extra_len = self
            .extra
            .keys()
            .filter(|key| key.as_str() != "index" && key.as_str() != "user")
            .count();
        let mut map = serializer.serialize_map(Some(
            1 + usize::from(self.user.is_some() || self.user_is_null) + extra_len,
        ))?;
        map.serialize_entry("index", &self.index_value())?;
        if let Some(user) = &self.user {
            map.serialize_entry("user", user)?;
        } else if self.user_is_null {
            map.serialize_entry("user", &Value::Null)?;
        }
        for (key, value) in &self.extra {
            if key == "index" || key == "user" {
                continue;
            }
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for PlaylistIndexPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut entries = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let index: Option<i64> = match entries.remove("index") {
            Some(Value::Null) | None => None,
            Some(value) => Some(serde_json::from_value(value).map_err(serde::de::Error::custom)?),
        };
        let mut payload = PlaylistIndexPayload::from_optional(index);
        if let Some(user_value) = entries.remove("user") {
            match user_value {
                Value::String(user) => payload.user = Some(user),
                Value::Null => payload.user_is_null = true,
                other => {
                    return Err(serde::de::Error::custom(format!(
                        "playlistIndex.user must be a string or null, got {other}"
                    )));
                }
            }
        }
        payload.extra.extend(entries);
        Ok(payload)
    }
}
