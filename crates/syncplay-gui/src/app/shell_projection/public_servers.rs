use super::*;

impl PublicServerBrowserShellState {
    pub(in crate::app) fn from_stored_settings(settings: &StoredClientSettingsMvp) -> Self {
        let servers = settings
            .public_servers
            .as_ref()
            .map(|entries| {
                entries
                    .iter()
                    .enumerate()
                    .map(|(index, (label, address))| PublicServerBrowserRow {
                        label: label.clone(),
                        address: address.clone(),
                        is_selected: index == 0,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Self {
            can_connect: !servers.is_empty(),
            can_refresh: true,
            can_add_custom_server: true,
            servers,
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn render_lines(&self) -> Vec<String> {
        let mut lines = vec![
            "[Public Server Browser]".to_owned(),
            format!(
                "Actions: connect={}, refresh={}, add_custom={}",
                bool_label(self.can_connect),
                bool_label(self.can_refresh),
                bool_label(self.can_add_custom_server),
            ),
            format!("Servers ({}):", self.servers.len()),
        ];

        if self.servers.is_empty() {
            lines.push("- (empty)".to_owned());
        } else {
            for server in &self.servers {
                lines.push(format!(
                    "- {} @ {} [selected={}]",
                    server.label,
                    server.address,
                    bool_label(server.is_selected),
                ));
            }
        }

        lines
    }

    pub(in crate::app) fn apply_runtime_flags(
        &mut self,
        runtime_flags: PublicServerBrowserRuntimeFlags,
    ) {
        self.can_connect = runtime_flags.can_connect && !self.servers.is_empty();
        self.can_refresh = runtime_flags.can_refresh;
        self.can_add_custom_server = runtime_flags.can_add_custom_server;
    }
}

impl PublicServerBrowserRuntimeFlags {
    pub(in crate::app) fn from_shell_state(state: &PublicServerBrowserShellState) -> Self {
        Self {
            can_connect: state.can_connect,
            can_refresh: state.can_refresh,
            can_add_custom_server: state.can_add_custom_server,
        }
    }
}
