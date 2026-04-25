use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProtocolMessage {
    Hello(HelloMessage),
    Set(Box<SetMessage>),
    List(Box<ListMessage>),
    State(Box<StateMessage>),
    Chat(ChatMessage),
    Error(ErrorMessage),
    Tls(TlsMessage),
}

impl ProtocolMessage {
    pub fn hello(hello: HelloPayload) -> Self {
        Self::Hello(HelloMessage::new(hello))
    }

    pub fn hello_basic(
        username: impl Into<String>,
        room_name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self::hello(HelloPayload::new(username, room_name, version))
    }

    pub fn set(set: SetPayload) -> Self {
        Self::Set(Box::new(SetMessage::new(set)))
    }

    pub fn list(list: ListPayload) -> Self {
        Self::List(Box::new(ListMessage::new(list)))
    }

    pub fn list_request() -> Self {
        Self::list(ListPayload::request())
    }

    pub fn state(state: StatePayload) -> Self {
        Self::State(Box::new(StateMessage::new(state)))
    }

    pub fn chat(chat: ChatPayload) -> Self {
        Self::Chat(ChatMessage::new(chat))
    }

    pub fn chat_text(text: impl Into<String>) -> Self {
        Self::chat(ChatPayload::text(text))
    }

    pub fn chat_message(username: impl Into<String>, message: impl Into<String>) -> Self {
        Self::chat(ChatPayload::message(username, message))
    }

    pub fn error(error: ErrorPayload) -> Self {
        Self::Error(ErrorMessage::new(error))
    }

    pub fn error_message(message: impl Into<String>) -> Self {
        Self::error(ErrorPayload::new(message))
    }

    pub fn tls(tls: TlsPayload) -> Self {
        Self::Tls(TlsMessage::new(tls))
    }

    pub fn start_tls(mode: impl Into<String>) -> Self {
        Self::tls(TlsPayload::new(mode))
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello(_) => "Hello",
            Self::Set(_) => "Set",
            Self::List(_) => "List",
            Self::State(_) => "State",
            Self::Chat(_) => "Chat",
            Self::Error(_) => "Error",
            Self::Tls(_) => "TLS",
        }
    }
}

impl From<HelloPayload> for ProtocolMessage {
    fn from(value: HelloPayload) -> Self {
        Self::hello(value)
    }
}

impl From<SetPayload> for ProtocolMessage {
    fn from(value: SetPayload) -> Self {
        Self::set(value)
    }
}

impl From<ListPayload> for ProtocolMessage {
    fn from(value: ListPayload) -> Self {
        Self::list(value)
    }
}

impl From<StatePayload> for ProtocolMessage {
    fn from(value: StatePayload) -> Self {
        Self::state(value)
    }
}

impl From<ChatPayload> for ProtocolMessage {
    fn from(value: ChatPayload) -> Self {
        Self::chat(value)
    }
}

impl From<ErrorPayload> for ProtocolMessage {
    fn from(value: ErrorPayload) -> Self {
        Self::error(value)
    }
}

impl From<TlsPayload> for ProtocolMessage {
    fn from(value: TlsPayload) -> Self {
        Self::tls(value)
    }
}
