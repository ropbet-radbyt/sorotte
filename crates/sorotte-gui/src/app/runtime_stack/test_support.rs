use super::GuiSessionRuntimeAdapter;

pub(in crate::app) trait GuiSessionDeliveryTestExt:
    GuiSessionRuntimeAdapter
{
    /// Capture completed writes through the production staging and receipt API.
    fn deliver_outbound_protocol_lines(&mut self) -> Result<Vec<String>, String> {
        let mut delivered = Vec::new();
        while let Some(frame) = self.begin_outbound_protocol_delivery()? {
            delivered.push(
                self.acknowledge_outbound_protocol_delivery(frame.token())?
                    .expect("the written frame should retain its receipt"),
            );
        }
        Ok(delivered)
    }
}

impl<T: GuiSessionRuntimeAdapter + ?Sized> GuiSessionDeliveryTestExt for T {}
