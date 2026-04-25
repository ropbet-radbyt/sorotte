use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ListPayload {
    Request(Option<()>),
    Rooms(BTreeMap<String, BTreeMap<String, ListUserEntry>>),
}

impl ListPayload {
    pub fn request() -> Self {
        Self::Request(None)
    }

    pub fn rooms(rooms: BTreeMap<String, BTreeMap<String, ListUserEntry>>) -> Self {
        Self::Rooms(rooms)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ListUserEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "isReady")]
    pub is_ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ListUserEntry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_position(mut self, position: f64) -> Self {
        self.position = Some(position);
        self
    }

    pub fn with_file(mut self, file: Value) -> Self {
        self.file = Some(file);
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

    pub fn with_features(mut self, features: Value) -> Self {
        self.features = Some(features);
        self
    }
}
