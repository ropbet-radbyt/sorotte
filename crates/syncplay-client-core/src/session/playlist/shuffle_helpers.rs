use super::super::*;

impl ClientSession {
    pub(in crate::session) fn capture_playlist_undo_snapshot_legacy_compatible(
        &mut self,
        room_name: &str,
        current_files: &[String],
        new_files: &[String],
    ) {
        if current_files == new_files {
            return;
        }
        if self
            .playlist_undo_snapshots
            .get(room_name)
            .is_some_and(|snapshot| snapshot == current_files)
        {
            return;
        }
        self.playlist_undo_snapshots
            .insert(room_name.to_owned(), current_files.to_vec());
    }

    pub(in crate::session) fn local_playlist_target_index_from_changed_playlist_legacy_compatible(
        current_files: &[String],
        current_index: Option<usize>,
        new_files: &[String],
    ) -> usize {
        let Some(current_index) = current_index else {
            return 0;
        };
        if new_files.len() <= 1 {
            return 0;
        }

        let mut index = current_index;
        while index <= current_files.len() {
            if let Some(file_name) = current_files.get(index)
                && let Some(valid_index) = new_files.iter().position(|entry| entry == file_name)
            {
                return valid_index;
            }
            index = index.saturating_add(1);
        }

        let mut index = current_index;
        while index > 0 {
            if let Some(file_name) = current_files.get(index)
                && let Some(valid_index) = new_files.iter().position(|entry| entry == file_name)
            {
                return if valid_index < new_files.len().saturating_sub(1) {
                    valid_index.saturating_add(1)
                } else {
                    valid_index
                };
            }
            index = index.saturating_sub(1);
        }
        0
    }

    pub(in crate::session) fn next_playlist_shuffle_seed_legacy_compatible(
        &mut self,
        files: &[String],
        current_index: usize,
        shuffle_scope_remaining: bool,
    ) -> u64 {
        let mut hasher = Sha256::new();
        hasher.update(if shuffle_scope_remaining {
            &b"remaining"[..]
        } else {
            &b"entire"[..]
        });
        hasher.update((current_index as u64).to_le_bytes());
        hasher.update(self.playlist_shuffle_nonce.to_le_bytes());
        for file_name in files {
            hasher.update(file_name.as_bytes());
            hasher.update([0]);
        }
        self.playlist_shuffle_nonce = self.playlist_shuffle_nonce.wrapping_add(1);

        let digest = hasher.finalize();
        let mut seed_bytes = [0u8; 8];
        seed_bytes.copy_from_slice(&digest[..8]);
        let seed = u64::from_le_bytes(seed_bytes);
        if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        }
    }

    pub(in crate::session) fn next_shuffle_state_legacy_compatible(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state
    }

    pub(in crate::session) fn shuffle_playlist_slice_in_place_legacy_compatible(
        files: &mut [String],
        seed: u64,
    ) {
        if files.len() <= 1 {
            return;
        }

        let mut state = seed;
        for index in (1..files.len()).rev() {
            let random_value = Self::next_shuffle_state_legacy_compatible(&mut state);
            let swap_index = (random_value as usize) % (index + 1);
            files.swap(index, swap_index);
        }
    }
}
