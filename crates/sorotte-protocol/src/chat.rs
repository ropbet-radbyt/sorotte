use super::*;
use crate::redacted_debug::{RedactedJsonMap, RedactedSensitiveText};

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

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessagePayload {
    pub username: String,
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for ChatMessagePayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChatMessagePayload")
            .field("username", &self.username)
            .field("message", &RedactedSensitiveText(&self.message))
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
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

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub message: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for ErrorPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ErrorPayload")
            .field("message", &sorotte_secret::REDACTED_SECRET)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl ErrorPayload {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct TlsPayload {
    #[serde(rename = "startTLS")]
    pub start_tls: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl std::fmt::Debug for TlsPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TlsPayload")
            .field("start_tls", &self.start_tls)
            .field("extra", &RedactedJsonMap(&self.extra))
            .finish()
    }
}

impl TlsPayload {
    pub fn new(start_tls: impl Into<String>) -> Self {
        Self {
            start_tls: start_tls.into(),
            extra: BTreeMap::new(),
        }
    }
}
