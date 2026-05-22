use super::super::*;

impl ClientSession {
    pub(in crate::session) fn loop_single_files_enabled_legacy_compatible(&self) -> bool {
        self.behavior_config.loop_single_files || self.is_playing_music()
    }

    pub(in crate::session) fn loop_at_end_of_playlist_enabled_legacy_compatible(&self) -> bool {
        self.behavior_config.loop_at_end_of_playlist || self.is_playing_music()
    }

    pub(in crate::session) fn playlist_target_switch_allowed_legacy_compatible(
        &self,
        file_name: &str,
    ) -> bool {
        if !Self::is_url(file_name) {
            return true;
        }
        self.uri_is_trusted_legacy_compatible(file_name)
    }

    pub(in crate::session) fn uri_is_trusted_legacy_compatible(&self, uri: &str) -> bool {
        let Some((host, path)) = Self::parse_trustable_web_uri_host_and_path_legacy_compatible(uri)
        else {
            return false;
        };

        if !self.behavior_config.only_switch_to_trusted_domains {
            return true;
        }

        for trusted_entry in &self.behavior_config.trusted_domains {
            let trusted_entry = trusted_entry.trim();
            if trusted_entry.is_empty() {
                continue;
            }
            let (trusted_domain, required_path_prefix) =
                trusted_entry.split_once('/').unwrap_or((trusted_entry, ""));
            let trusted_domain = trusted_domain.trim().to_ascii_lowercase();
            if trusted_domain.is_empty() {
                continue;
            }
            if !Self::trusted_domain_matches_host_legacy_compatible(&host, &trusted_domain) {
                continue;
            }
            if !required_path_prefix.is_empty() {
                let path_prefix = format!("/{required_path_prefix}");
                if !path.starts_with(&path_prefix) {
                    continue;
                }
            }
            return true;
        }
        false
    }

    pub(in crate::session) fn parse_trustable_web_uri_host_and_path_legacy_compatible(
        uri: &str,
    ) -> Option<(String, String)> {
        let uri = uri.trim();
        let authority_and_path = if let Some(value) = uri.strip_prefix("http://") {
            value
        } else {
            uri.strip_prefix("https://")?
        };
        if authority_and_path.is_empty() {
            return None;
        }

        let (authority, path_tail) = authority_and_path
            .split_once('/')
            .unwrap_or((authority_and_path, ""));
        if authority.is_empty() {
            return None;
        }

        let authority = authority
            .rsplit_once('@')
            .map_or(authority, |(_, value)| value);
        if authority.is_empty() {
            return None;
        }

        let host = authority
            .split(':')
            .next()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }

        let path_with_query = if path_tail.is_empty() {
            "/".to_owned()
        } else {
            format!("/{path_tail}")
        };
        let path = path_with_query
            .split(['?', '#'])
            .next()
            .unwrap_or("/")
            .to_owned();
        Some((host, path))
    }

    pub(in crate::session) fn trusted_domain_matches_host_legacy_compatible(
        host: &str,
        trusted_domain: &str,
    ) -> bool {
        if host == trusted_domain || host == format!("www.{trusted_domain}") {
            return true;
        }
        if !trusted_domain.contains('*') {
            return false;
        }

        let host_parts = host.split('.').collect::<Vec<_>>();
        let pattern_parts = trusted_domain.split('.').collect::<Vec<_>>();
        if host_parts.len() != pattern_parts.len() {
            return false;
        }
        host_parts
            .iter()
            .zip(pattern_parts.iter())
            .all(|(host_part, pattern_part)| {
                if *pattern_part == "*" {
                    !host_part.is_empty()
                } else {
                    host_part.eq_ignore_ascii_case(pattern_part)
                }
            })
    }
}
