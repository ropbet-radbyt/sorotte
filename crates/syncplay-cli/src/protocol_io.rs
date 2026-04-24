use syncplay_client_core::{ClientRuntime, QueuedRuntimeControl};
use syncplay_player_mpv::MpvAdapter;
use tokio::io::AsyncWriteExt;
pub(crate) async fn write_protocol_line(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    line: &str,
) -> anyhow::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

pub(super) async fn flush_runtime_protocol_lines(
    runtime: &mut ClientRuntime<MpvAdapter, QueuedRuntimeControl>,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
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
