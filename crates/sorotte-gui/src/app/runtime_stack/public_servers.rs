use std::collections::BTreeSet;

use sorotte_client_app::app_boundary::{
    persistence::parse_serialized_public_servers_list_legacy_compatible,
    state::parse_host_and_optional_port_from_host_arg_legacy_compatible,
};

use super::super::startup_support::env_trimmed;
use super::super::support::normalized_editable_text;
use super::GuiClientCoreChatSessionRuntimeAdapter;

impl GuiClientCoreChatSessionRuntimeAdapter {
    pub(in crate::app) fn normalize_public_server_rows(
        current_servers: Vec<(String, String)>,
    ) -> Vec<(String, String)> {
        let mut normalized = Vec::new();
        let mut seen_addresses = BTreeSet::new();
        for (label, address) in current_servers {
            let Some(label) = normalized_editable_text(&label) else {
                continue;
            };
            let Some(address) = normalized_editable_text(&address) else {
                continue;
            };
            let (host, _) = parse_host_and_optional_port_from_host_arg_legacy_compatible(&address);
            if host.trim().is_empty() {
                continue;
            }
            let dedupe_key = address.to_ascii_lowercase();
            if !seen_addresses.insert(dedupe_key) {
                continue;
            }
            normalized.push((label, address));
        }
        normalized
    }

    pub(in crate::app) fn refreshed_public_server_rows_from_lookup<F>(
        lookup: &F,
    ) -> Result<Option<Vec<(String, String)>>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_name = "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS";
        let Some(value) = lookup(env_name) else {
            return Ok(None);
        };
        let Some(parsed) = parse_serialized_public_servers_list_legacy_compatible(&value) else {
            return Err(format!(
                "{env_name} must be a serialized public-server list like [[\"Primary\", \"syncplay.pl:8999\"]]."
            ));
        };
        Ok(Some(Self::normalize_public_server_rows(parsed)))
    }

    pub(in crate::app) fn refreshed_public_server_rows_from_sources<F, R>(
        lookup: &F,
        read_to_string: &R,
    ) -> Result<Option<Vec<(String, String)>>, String>
    where
        F: Fn(&str) -> Option<String>,
        R: Fn(&str) -> Result<String, String>,
    {
        let path_env_name = "SOROTTE_GUI_REFRESH_PUBLIC_SERVERS_PATH";
        if let Some(path) = lookup(path_env_name) {
            let value = read_to_string(&path)
                .map_err(|error| format!("{path_env_name} could not read '{path}': {error}"))?;
            let Some(parsed) = parse_serialized_public_servers_list_legacy_compatible(&value)
            else {
                return Err(format!(
                    "{path_env_name} file '{path}' must be a serialized public-server list like [[\"Primary\", \"syncplay.pl:8999\"]]."
                ));
            };
            return Ok(Some(Self::normalize_public_server_rows(parsed)));
        }

        Self::refreshed_public_server_rows_from_lookup(lookup)
    }

    pub(in crate::app) fn refreshed_public_server_rows_from_env()
    -> Result<Option<Vec<(String, String)>>, String> {
        Self::refreshed_public_server_rows_from_sources(&env_trimmed, &|path| {
            std::fs::read_to_string(path).map_err(|error| error.to_string())
        })
    }
}
