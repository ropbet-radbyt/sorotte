use std::path::Path;

use syncplay_player_api::{LocalFileUpdate, PlayerAdapter, PlayerError};

#[derive(Debug, Default)]
pub struct MpvAdapter {
    paused: bool,
    position_seconds: f64,
    playback_rate: f64,
    current_path: Option<String>,
    pending_local_file_update: Option<LocalFileUpdate>,
}

impl MpvAdapter {
    pub fn current_path(&self) -> Option<&str> {
        self.current_path.as_deref()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn position_seconds(&self) -> f64 {
        self.position_seconds
    }

    pub fn playback_rate(&self) -> f64 {
        if self.playback_rate == 0.0 {
            1.0
        } else {
            self.playback_rate
        }
    }

    pub fn queue_local_file_update(&mut self, update: LocalFileUpdate) {
        self.pending_local_file_update = Some(update);
    }

    fn local_file_update_for_path(path: &str) -> LocalFileUpdate {
        let name = if path.contains("://") {
            path.to_owned()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(path)
                .to_owned()
        };
        let size_bytes = if path.contains("://") {
            0
        } else {
            std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
        };

        LocalFileUpdate::new(name)
            .with_size_bytes(size_bytes)
            .with_path(path.to_owned())
    }
}

impl PlayerAdapter for MpvAdapter {
    fn name(&self) -> &'static str {
        "mpv"
    }

    fn open_file(&mut self, path: &str) -> Result<(), PlayerError> {
        self.current_path = Some(path.to_owned());
        self.pending_local_file_update = Some(Self::local_file_update_for_path(path));
        Ok(())
    }

    fn set_paused(&mut self, paused: bool) -> Result<(), PlayerError> {
        self.paused = paused;
        Ok(())
    }

    fn set_position(&mut self, position_seconds: f64) -> Result<(), PlayerError> {
        self.position_seconds = position_seconds;
        Ok(())
    }

    fn set_playback_rate(&mut self, rate: f64) -> Result<(), PlayerError> {
        self.playback_rate = rate;
        Ok(())
    }

    fn take_local_file_update(&mut self) -> Option<LocalFileUpdate> {
        self.pending_local_file_update.take()
    }
}

#[cfg(test)]
mod tests {
    use super::MpvAdapter;
    use std::{fs::File, io::Write};
    use syncplay_player_api::{LocalFileUpdate, PlayerAdapter};

    #[test]
    fn stores_opened_file_path() {
        let mut adapter = MpvAdapter::default();
        adapter
            .open_file("movie.mkv")
            .expect("mpv stub should accept file");
        assert_eq!(adapter.current_path(), Some("movie.mkv"));

        let file_update = adapter
            .take_local_file_update()
            .expect("open file should produce local file update");
        assert_eq!(file_update.name, "movie.mkv");
        assert_eq!(file_update.path.as_deref(), Some("movie.mkv"));
    }

    #[test]
    fn stores_runtime_state_updates() {
        let mut adapter = MpvAdapter::default();
        adapter
            .set_paused(true)
            .expect("mpv stub should accept paused updates");
        adapter
            .set_position(24.5)
            .expect("mpv stub should accept position updates");
        adapter
            .set_playback_rate(0.95)
            .expect("mpv stub should accept speed updates");

        assert!(adapter.paused());
        assert_eq!(adapter.position_seconds(), 24.5);
        assert_eq!(adapter.playback_rate(), 0.95);
    }

    #[test]
    fn queue_local_file_update_is_drained_once() {
        let mut adapter = MpvAdapter::default();
        adapter.queue_local_file_update(
            LocalFileUpdate::new("movie.mkv")
                .with_duration_seconds(95.5)
                .with_size_bytes(123),
        );

        let first = adapter
            .take_local_file_update()
            .expect("queued local file update should be returned");
        assert_eq!(first.name, "movie.mkv");
        assert_eq!(first.duration_seconds, Some(95.5));
        assert_eq!(first.size_bytes, Some(123));
        assert_eq!(adapter.take_local_file_update(), None);
    }

    #[test]
    fn open_file_collects_filesystem_size_for_local_paths() {
        let temp_path = std::env::temp_dir().join("syncplay_mpv_adapter_size_probe.tmp");
        let mut temp_file = File::create(&temp_path).expect("temp file should be creatable");
        writeln!(temp_file, "12345").expect("temp file should be writable");
        drop(temp_file);

        let mut adapter = MpvAdapter::default();
        adapter
            .open_file(temp_path.to_string_lossy().as_ref())
            .expect("mpv stub should accept local temp file");

        let file_update = adapter
            .take_local_file_update()
            .expect("open file should queue local file metadata update");
        assert_eq!(
            file_update.path.as_deref(),
            Some(temp_path.to_string_lossy().as_ref())
        );
        assert!(
            file_update.size_bytes.is_some_and(|size| size >= 6),
            "expected local file metadata size"
        );

        std::fs::remove_file(temp_path).expect("temp file should be removable");
    }
}
