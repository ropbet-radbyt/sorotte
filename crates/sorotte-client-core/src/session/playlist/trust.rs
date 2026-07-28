use super::super::*;
use url::Url;

const TRUSTED_DOMAIN_WILDCARD_PLACEHOLDER_PREFIX: &str = "sorotte-wildcard-placeholder";

struct TrustableWebUrl {
    scheme: String,
    host: String,
    explicit_port: Option<u16>,
    effective_port: u16,
    path_segments: Vec<String>,
}

struct TrustedWebUrlPattern {
    scheme: Option<String>,
    host_pattern: String,
    explicit_port: Option<u16>,
    path_segments: Vec<String>,
}

fn decode_canonical_path_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        let high = *bytes.get(index + 1)?;
        let low = *bytes.get(index + 2)?;
        let hex = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        };
        decoded.push(hex(high)? * 16 + hex(low)?);
        index += 3;
    }
    let decoded = String::from_utf8(decoded).ok()?;
    if decoded == "."
        || decoded == ".."
        || decoded
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return None;
    }
    Some(decoded)
}

fn canonical_path_segments(url: &Url) -> Option<Vec<String>> {
    let mut segments = url
        .path_segments()?
        .map(decode_canonical_path_segment)
        .collect::<Option<Vec<_>>>()?;
    if segments.last().is_some_and(String::is_empty) {
        segments.pop();
    }
    Some(segments)
}

fn parse_trustable_web_url(uri: &str) -> Option<TrustableWebUrl> {
    let url = Url::parse(uri.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let effective_port = url.port_or_known_default()?;
    Some(TrustableWebUrl {
        scheme: url.scheme().to_owned(),
        host,
        explicit_port: url.port(),
        effective_port,
        path_segments: canonical_path_segments(&url)?,
    })
}

fn parse_trusted_web_url_pattern(entry: &str) -> Option<TrustedWebUrlPattern> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    let has_explicit_scheme = entry.contains("://");
    let lowercase_entry = entry.to_ascii_lowercase();
    let wildcard_placeholder = (0_u64..).find_map(|suffix| {
        let candidate = format!("{TRUSTED_DOMAIN_WILDCARD_PLACEHOLDER_PREFIX}-{suffix}");
        (!lowercase_entry.contains(&candidate)).then_some(candidate)
    })?;
    let parseable = if has_explicit_scheme {
        entry.to_owned()
    } else {
        format!("https://{entry}")
    }
    .replace('*', &wildcard_placeholder);
    let url = Url::parse(&parseable).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let host_pattern = url
        .host_str()?
        .to_ascii_lowercase()
        .replace(&wildcard_placeholder, "*");
    if host_pattern
        .split('.')
        .any(|label| label.contains('*') && label != "*")
    {
        return None;
    }
    let explicit_port = if has_explicit_scheme {
        url.port()
    } else {
        Url::parse(&format!("sorotte-trusted-domain://{entry}").replace('*', &wildcard_placeholder))
            .ok()?
            .port()
    };
    Some(TrustedWebUrlPattern {
        scheme: (has_explicit_scheme || explicit_port.is_some()).then(|| url.scheme().to_owned()),
        host_pattern,
        explicit_port,
        path_segments: canonical_path_segments(&url)?,
    })
}

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
        if Self::is_plex_uri(file_name) {
            return true;
        }
        if !Self::is_url(file_name) {
            return true;
        }
        self.uri_is_trusted_legacy_compatible(file_name)
    }

    pub(in crate::session) fn uri_is_trusted_legacy_compatible(&self, uri: &str) -> bool {
        playback_uri_is_trusted_legacy_compatible(
            uri,
            self.behavior_config.only_switch_to_trusted_domains,
            &self.behavior_config.trusted_domains,
        )
    }
}

pub fn playback_uri_is_trusted_legacy_compatible(
    uri: &str,
    only_switch_to_trusted_domains: bool,
    trusted_domains: &[String],
) -> bool {
    if !only_switch_to_trusted_domains {
        return Url::parse(uri.trim())
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    }
    let Some(target) = parse_trustable_web_url(uri) else {
        return false;
    };

    for trusted_entry in trusted_domains {
        let Some(pattern) = parse_trusted_web_url_pattern(trusted_entry) else {
            continue;
        };
        if pattern
            .scheme
            .as_deref()
            .is_some_and(|scheme| scheme != target.scheme)
        {
            continue;
        }
        if !trusted_domain_matches_host_legacy_compatible(&target.host, &pattern.host_pattern) {
            continue;
        }
        let port_matches = pattern.explicit_port.map_or_else(
            || target.explicit_port.is_none(),
            |port| target.effective_port == port,
        );
        if !port_matches {
            continue;
        }
        if pattern.path_segments.len() > target.path_segments.len()
            || !pattern
                .path_segments
                .iter()
                .zip(&target.path_segments)
                .all(|(required, actual)| required == actual)
        {
            continue;
        }
        return true;
    }
    false
}

fn trusted_domain_matches_host_legacy_compatible(host: &str, trusted_domain: &str) -> bool {
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
