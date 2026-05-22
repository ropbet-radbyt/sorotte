use super::*;

impl MediaSearchWorkflowShellState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let directories = settings
            .media_search_directories
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|path| MediaSearchDirectoryRow {
                path,
                is_selected: false,
            })
            .collect::<Vec<_>>();

        Self {
            can_browse_directories: true,
            can_search_missing_media: !directories.is_empty(),
            first_file_timeout_seconds: settings.folder_search_first_file_timeout_seconds,
            search_timeout_seconds: settings.folder_search_timeout_seconds,
            double_check_interval_seconds: settings.folder_search_double_check_interval_seconds,
            warning_threshold_seconds: settings.folder_search_warning_threshold_seconds,
            directories,
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "[Media Search Workflow]".to_owned(),
            format!(
                "Actions: browse_directories={}, search_missing_media={}",
                bool_label(self.can_browse_directories),
                bool_label(self.can_search_missing_media),
            ),
            format!(
                "Timing: first_file={}, search={}, double_check={}, warning={}",
                optional_seconds_text(self.first_file_timeout_seconds),
                optional_seconds_text(self.search_timeout_seconds),
                optional_seconds_text(self.double_check_interval_seconds),
                optional_seconds_text(self.warning_threshold_seconds),
            ),
            format!("Directories ({}):", self.directories.len()),
        ];

        if self.directories.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for directory in &self.directories {
                lines.push(format!(
                    "- {} [selected={}]",
                    directory.path,
                    bool_label(directory.is_selected),
                ));
            }
        }

        lines
    }

    pub(in crate::app) fn apply_runtime_flags(
        &mut self,
        runtime_flags: MediaSearchWorkflowRuntimeFlags,
    ) {
        self.can_browse_directories = runtime_flags.can_browse_directories;
        self.can_search_missing_media =
            runtime_flags.can_search_missing_media && !self.directories.is_empty();
    }
}

impl MediaSearchWorkflowRuntimeFlags {
    pub(in crate::app) fn from_shell_state(state: &MediaSearchWorkflowShellState) -> Self {
        Self {
            can_browse_directories: state.can_browse_directories,
            can_search_missing_media: state.can_search_missing_media,
        }
    }
}
