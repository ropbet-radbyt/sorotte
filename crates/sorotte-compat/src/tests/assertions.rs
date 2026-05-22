use super::*;

fn is_null_playlist_index_protocol_message(message: &ProtocolMessage) -> bool {
    match message {
        ProtocolMessage::Set(payload) => payload
            .set
            .playlist_index
            .as_ref()
            .is_some_and(|playlist_index| playlist_index.index_value().is_none()),
        _ => false,
    }
}

mod legacy_client_assertions;
mod legacy_fanout_assertions;
mod legacy_process_assertions;
mod legacy_tls_assertions;
mod python_fanout_assertions;
mod tls_io_assertions;
mod trace_assertions;

pub(super) use legacy_client_assertions::*;
pub(super) use legacy_fanout_assertions::*;
pub(super) use legacy_process_assertions::*;
pub(super) use legacy_tls_assertions::*;
pub(super) use python_fanout_assertions::*;
pub(super) use tls_io_assertions::*;
pub(super) use trace_assertions::*;
