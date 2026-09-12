use crate::{ClientRuntime, QueuedRuntimeControl};
use sorotte_player_api::PlayerAdapter;
use sorotte_protocol::{ProtocolError, ProtocolMessage};

impl<P: PlayerAdapter> ClientRuntime<P, QueuedRuntimeControl> {
    /// Model successful writes. Use `control().outbound_messages()` when a test
    /// needs to inspect pending, still-coalescible work without delivering it.
    pub(crate) fn deliver_queued_protocol_messages(&mut self) -> Vec<ProtocolMessage> {
        let mut delivered = Vec::new();
        while let Some(pending) = self.pending_protocol_line().expect("frame should encode") {
            delivered.push(
                self.acknowledge_protocol_line(pending.lease())
                    .expect("the written frame should retain its lease"),
            );
        }
        delivered
    }

    pub(crate) fn deliver_queued_protocol_lines(&mut self) -> Result<Vec<String>, ProtocolError> {
        let mut delivered = Vec::new();
        self.flush_queued_protocol_lines_to_transport(|line| {
            delivered.push(line.to_owned());
            Ok(())
        })?;
        Ok(delivered)
    }
}
