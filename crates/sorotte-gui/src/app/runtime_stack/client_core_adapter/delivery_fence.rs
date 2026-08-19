use super::*;

impl GuiClientCoreChatSessionRuntimeAdapter {
    pub(super) fn pending_playlist_protocol_delivery_fence(
        &self,
    ) -> Result<GuiPlaylistProtocolDeliveryFence, String> {
        let pending_playlist_lines = self
            .runtime
            .pending_protocol_messages()
            .iter()
            .filter(|message| {
                matches!(
                    message,
                    ProtocolMessage::Set(set)
                        if set.set.playlist_change.is_some() || set.set.playlist_index.is_some()
                )
            })
            .map(encode_message_line)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                format!("Queued playlist delivery-fence line encoding failed: {error}")
            })?;
        Ok(GuiPlaylistProtocolDeliveryFence::new(
            pending_playlist_lines,
        ))
    }
}
