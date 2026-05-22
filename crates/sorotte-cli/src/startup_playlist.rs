use std::path::Path;

use anyhow::anyhow;
use sorotte_protocol::{
    PlaylistChangePayload, PlaylistIndexPayload, ProtocolMessage, SetPayload, encode_message_line,
};
use tokio::io::AsyncWrite;
pub(super) fn protocol_lines_for_startup_playlist_load_from_file_legacy_compatible(
    path: &Path,
) -> anyhow::Result<Vec<String>> {
    if !path.is_file() {
        eprintln!(
            "warning: legacy --load-playlist-from-file skipped because file was not found: {}",
            path.display()
        );
        return Ok(Vec::new());
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("failed reading playlist file {}: {error}", path.display()))?;
    let files = contents.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if files.is_empty() {
        return Ok(Vec::new());
    }

    let playlist_change_message = ProtocolMessage::set(
        SetPayload::new().with_playlist_change(PlaylistChangePayload::new(files)),
    );
    let playlist_index_message =
        ProtocolMessage::set(SetPayload::new().with_playlist_index(PlaylistIndexPayload::new(0)));
    Ok(vec![
        encode_message_line(&playlist_change_message)?,
        encode_message_line(&playlist_index_message)?,
    ])
}

pub(super) async fn emit_startup_playlist_load_from_file_legacy_compatible(
    writer: &mut (impl AsyncWrite + Unpin),
    playlist_path: &str,
) -> anyhow::Result<bool> {
    let lines = protocol_lines_for_startup_playlist_load_from_file_legacy_compatible(Path::new(
        playlist_path,
    ))?;
    if lines.is_empty() {
        return Ok(false);
    }
    for line in &lines {
        crate::protocol_io::write_protocol_line(writer, line).await?;
    }
    Ok(true)
}
