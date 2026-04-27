use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod chat;
mod codec;
mod envelope;
mod hello;
mod list;
mod message;
mod room;
mod set;
mod state;

pub use chat::{ChatMessagePayload, ChatPayload, ErrorPayload, TlsPayload};
pub use codec::{
    DecodedMessageLineItem, ProtocolError, decode_line, decode_message_line,
    decode_message_line_items, decode_message_lines, encode_line, encode_message_line,
    extract_hello, extract_hello_from_message,
};
pub use envelope::{
    ChatMessage, ErrorMessage, HelloMessage, ListMessage, SetMessage, StateMessage, TlsMessage,
};
pub use hello::HelloPayload;
pub use list::{ListPayload, ListUserEntry};
pub use message::ProtocolMessage;
pub use room::RoomRef;
pub use set::{
    ControllerAuthPayload, FilePayload, NewControlledRoomPayload, PlaylistChangePayload,
    PlaylistIndexPayload, ReadyPayload, SetPayload, UserSetPayload,
};
pub use state::{IgnoringOnTheFlyPayload, PingPayload, PlaystatePayload, StatePayload};

#[cfg(test)]
mod tests;
