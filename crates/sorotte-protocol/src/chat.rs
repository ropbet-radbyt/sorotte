use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatPayload {
    Text(String),
    Message(ChatMessagePayload),
}

impl ChatPayload {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn message(username: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Message(ChatMessagePayload::new(username, message))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessagePayload {
    pub username: String,
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ChatMessagePayload {
    pub fn new(username: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            message: message.into(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl ErrorPayload {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TlsPayload {
    #[serde(rename = "startTLS")]
    pub start_tls: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl TlsPayload {
    pub fn new(start_tls: impl Into<String>) -> Self {
        Self {
            start_tls: start_tls.into(),
            extra: BTreeMap::new(),
        }
    }
}
