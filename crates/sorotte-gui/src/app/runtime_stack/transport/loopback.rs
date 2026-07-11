use super::handle::{
    GuiOutboundProtocolDeliveryResult, GuiQueuedSessionTransportHandle, GuiSessionTransportDriver,
};

pub(in crate::app) struct GuiLoopbackSessionTransportDriver {
    echo_username: String,
}

impl GuiLoopbackSessionTransportDriver {
    pub(in crate::app) fn new(echo_username: impl Into<String>) -> Self {
        Self {
            echo_username: echo_username.into(),
        }
    }

    fn json_string_literal(input: &str) -> Option<&str> {
        let mut characters = input.char_indices();
        match characters.next() {
            Some((_, '"')) => {}
            _ => return None,
        }

        let mut escaped = false;
        for (index, character) in characters {
            if escaped {
                escaped = false;
                continue;
            }
            match character {
                '\\' => escaped = true,
                '"' => return Some(&input[..=index]),
                _ => {}
            }
        }
        None
    }

    fn chat_message_literal(line: &str) -> Option<&str> {
        let rest = line.strip_prefix("{\"Chat\":")?.strip_suffix('}')?;
        if rest.starts_with('"') {
            return Self::json_string_literal(rest);
        }

        let message_key = "\"message\":";
        let message_index = rest.find(message_key)?;
        let message_start = message_index + message_key.len();
        Self::json_string_literal(rest.get(message_start..)?)
    }

    fn translated_inbound_line(&self, outbound_line: &str) -> String {
        let Some(message_literal) = Self::chat_message_literal(outbound_line) else {
            return outbound_line.to_owned();
        };
        format!(
            r#"{{"Chat":{{"username":{:?},"message":{message_literal}}}}}"#,
            self.echo_username
        )
    }
}

impl GuiSessionTransportDriver for GuiLoopbackSessionTransportDriver {
    fn pump(&mut self, transport: &GuiQueuedSessionTransportHandle) -> Result<(), String> {
        if let Some(delivery) = transport.take_outbound_protocol_delivery_for_driver() {
            let inbound_line = self.translated_inbound_line(&delivery.line);
            transport.push_inbound_protocol_line(inbound_line);
            transport.publish_outbound_protocol_delivery_result(
                GuiOutboundProtocolDeliveryResult::FrameWritten {
                    token: delivery.token,
                },
            );
        }

        let outbound_protocol_lines = transport.drain_outbound_protocol_lines();
        if outbound_protocol_lines.is_empty() {
            return Ok(());
        }
        transport.push_inbound_protocol_lines(
            outbound_protocol_lines
                .into_iter()
                .map(|line| self.translated_inbound_line(&line)),
        );
        Ok(())
    }
}
