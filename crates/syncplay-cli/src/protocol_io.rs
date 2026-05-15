use syncplay_client_core::{ClientRuntime, QueuedRuntimeControl};
use syncplay_player_mpv::MpvAdapter;
use syncplay_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) const MAX_INBOUND_PROTOCOL_LINE_BYTES: usize = DEFAULT_MAX_PROTOCOL_LINE_BYTES;

pub(crate) async fn read_inbound_protocol_line<R>(reader: &mut R) -> anyhow::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }

        if let Some(newline_index) = available.iter().position(|byte| *byte == b'\n') {
            let raw_line_len = line.len() + newline_index;
            let line_len = if newline_index == 0 {
                raw_line_len.saturating_sub(usize::from(line.last() == Some(&b'\r')))
            } else {
                raw_line_len.saturating_sub(usize::from(available[newline_index - 1] == b'\r'))
            };
            if line_len > MAX_INBOUND_PROTOCOL_LINE_BYTES {
                return Err(anyhow::anyhow!(
                    "Inbound protocol line too long: exceeded {} bytes",
                    MAX_INBOUND_PROTOCOL_LINE_BYTES
                ));
            }

            let take = newline_index + 1;
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            break;
        }

        if line.len() + available.len() > MAX_INBOUND_PROTOCOL_LINE_BYTES {
            return Err(anyhow::anyhow!(
                "Inbound protocol line too long: exceeded {} bytes",
                MAX_INBOUND_PROTOCOL_LINE_BYTES
            ));
        }

        let take = available.len();
        line.extend_from_slice(available);
        reader.consume(take);
    }

    if line.last() == Some(&b'\n') {
        line.pop();
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some(String::from_utf8(line)?))
}

pub(crate) async fn write_protocol_line<W>(writer: &mut W, line: &str) -> anyhow::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\r\n").await?;
    writer.flush().await?;
    Ok(())
}

pub(super) async fn flush_runtime_protocol_lines(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    writer: &mut (impl AsyncWrite + Unpin),
) -> anyhow::Result<()> {
    let mut lines = Vec::new();
    runtime.flush_queued_protocol_lines_to_transport(|line| {
        lines.push(line.to_owned());
        Ok(())
    })?;
    for line in &lines {
        write_protocol_line(writer, line).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncplay_protocol::decode_message_line_items;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn cli_connected_session_rejects_inbound_line_over_max_bytes() {
        let input = vec![b'a'; MAX_INBOUND_PROTOCOL_LINE_BYTES + 1];
        let mut reader = BufReader::new(&input[..]);

        let error = read_inbound_protocol_line(&mut reader)
            .await
            .expect_err("oversized inbound line should fail");

        assert!(
            error.to_string().contains("Inbound protocol line too long"),
            "oversized inbound line should produce a clear error"
        );
    }

    #[tokio::test]
    async fn cli_connected_session_accepts_batched_valid_line() {
        let input = br#"{"Chat":"hello","List":null}"#.to_vec();
        let mut framed = input.clone();
        framed.extend_from_slice(b"\r\n");
        let mut reader = BufReader::new(&framed[..]);

        let line = read_inbound_protocol_line(&mut reader)
            .await
            .expect("batched line read should succeed")
            .expect("batched line should be present");

        assert_eq!(line.as_bytes(), input);
        let items = decode_message_line_items(&line).expect("batched line should decode");
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn write_protocol_line_uses_crlf_framing() {
        let mut output = Vec::new();

        write_protocol_line(&mut output, r#"{"List":null}"#)
            .await
            .expect("protocol line should write");

        assert_eq!(output, b"{\"List\":null}\r\n");
    }
}
