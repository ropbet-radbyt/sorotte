use super::*;
use crate::redacted_debug::{RedactedJsonMap, RedactedOptionalJsonValue};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloPayload {
    pub username: String,
    pub room: RoomRef,
    pub version: String,
    #[serde(default)]
    pub realversion: Option<String>,
    #[serde(default)]
    pub features: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for HelloPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HelloPayload")
            .field("username", &self.username)
            .field("room", &self.room)
            .field("version", &self.version)
            .field("realversion", &self.realversion)
            .field(
                "features",
                &RedactedOptionalJsonValue(self.features.as_ref()),
            )
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl HelloPayload {
    pub fn new(
        username: impl Into<String>,
        room_name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            username: username.into(),
            room: RoomRef::new(room_name),
            version: version.into(),
            realversion: None,
            features: None,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_realversion(mut self, realversion: impl Into<String>) -> Self {
        self.realversion = Some(realversion.into());
        self
    }

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }

    pub fn effective_version(&self) -> &str {
        self.realversion.as_deref().unwrap_or(self.version.as_str())
    }
}
