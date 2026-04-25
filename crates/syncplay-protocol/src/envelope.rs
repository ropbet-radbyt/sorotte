use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloMessage {
    #[serde(rename = "Hello")]
    pub hello: HelloPayload,
}

impl HelloMessage {
    pub fn new(hello: HelloPayload) -> Self {
        Self { hello }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetMessage {
    #[serde(rename = "Set")]
    pub set: SetPayload,
}

impl SetMessage {
    pub fn new(set: SetPayload) -> Self {
        Self { set }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListMessage {
    #[serde(rename = "List")]
    pub list: ListPayload,
}

impl ListMessage {
    pub fn new(list: ListPayload) -> Self {
        Self { list }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateMessage {
    #[serde(rename = "State")]
    pub state: StatePayload,
}

impl StateMessage {
    pub fn new(state: StatePayload) -> Self {
        Self { state }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(rename = "Chat")]
    pub chat: ChatPayload,
}

impl ChatMessage {
    pub fn new(chat: ChatPayload) -> Self {
        Self { chat }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorMessage {
    #[serde(rename = "Error")]
    pub error: ErrorPayload,
}

impl ErrorMessage {
    pub fn new(error: ErrorPayload) -> Self {
        Self { error }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TlsMessage {
    #[serde(rename = "TLS")]
    pub tls: TlsPayload,
}

impl TlsMessage {
    pub fn new(tls: TlsPayload) -> Self {
        Self { tls }
    }
}
