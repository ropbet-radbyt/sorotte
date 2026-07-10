use sorotte_client_core::FileSize;
use sorotte_plex::is_plex_playlist_uri;

use super::super::support::normalized_editable_text;
use super::GuiStreamTargetKind;

pub(in crate::app) fn browser_is_web_url(value: &str) -> bool {
    let value = value.trim_start();
    value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        || value
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
}

pub(in crate::app) fn browser_is_plex_uri(value: &str) -> bool {
    is_plex_playlist_uri(value)
}

pub(in crate::app) fn browser_is_url(value: &str) -> bool {
    value.contains("://") && !browser_is_plex_uri(value)
}

pub(in crate::app) fn browser_domain_from_url(value: &str) -> Option<String> {
    if !browser_is_web_url(value) {
        return None;
    }
    reqwest::Url::parse(value).ok().and_then(|url| {
        url.host_str()
            .map(|host| host.strip_prefix("www.").unwrap_or(host).to_owned())
    })
}

pub(in crate::app) fn browser_parse_trustable_web_uri_host_and_path(
    value: &str,
) -> Option<(String, String)> {
    let value = value.trim();
    let authority_and_path = if let Some(rest) = value.strip_prefix("http://") {
        rest
    } else {
        value.strip_prefix("https://")?
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
        .map_or(authority, |(_, trimmed)| trimmed);
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

pub(in crate::app) fn browser_trusted_domain_matches_host(
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

pub(in crate::app) fn browser_uri_is_trusted(
    uri: &str,
    only_switch_to_trusted_domains: bool,
    trusted_domains: &[String],
) -> bool {
    if !browser_is_web_url(uri) {
        return true;
    }
    let Some((host, path)) = browser_parse_trustable_web_uri_host_and_path(uri) else {
        return false;
    };
    if !only_switch_to_trusted_domains {
        return true;
    }
    trusted_domains.iter().any(|entry| {
        let entry = entry.trim();
        if entry.is_empty() {
            return false;
        }
        let (trusted_domain, required_path_prefix) = entry.split_once('/').unwrap_or((entry, ""));
        let trusted_domain = trusted_domain.trim().to_ascii_lowercase();
        if trusted_domain.is_empty() || !browser_trusted_domain_matches_host(&host, &trusted_domain)
        {
            return false;
        }
        if required_path_prefix.is_empty() {
            return true;
        }
        path.starts_with(&format!("/{required_path_prefix}"))
    })
}

fn browser_media_url_path_looks_direct(path: &str) -> bool {
    let normalized = path.to_ascii_lowercase();
    [
        ".mp4", ".m4v", ".mkv", ".webm", ".avi", ".mov", ".mp3", ".aac", ".ogg", ".flac", ".wav",
        ".m3u8", ".mpd", ".ts",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

pub(in crate::app) fn browser_stream_target_kind(
    value: &str,
    trust_policy: Option<(bool, &[String])>,
) -> GuiStreamTargetKind {
    if browser_is_plex_uri(value) {
        return GuiStreamTargetKind::PlexUri;
    }
    if !browser_is_url(value) {
        return GuiStreamTargetKind::LocalPath;
    }

    if let Some((only_switch_to_trusted_domains, trusted_domains)) = trust_policy
        && browser_is_web_url(value)
        && !browser_uri_is_trusted(value, only_switch_to_trusted_domains, trusted_domains)
    {
        return GuiStreamTargetKind::UntrustedUrl;
    }

    let Ok(parsed) = reqwest::Url::parse(value) else {
        return GuiStreamTargetKind::DirectMediaUrl;
    };
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https") {
        return GuiStreamTargetKind::DirectMediaUrl;
    }

    let host = parsed
        .host_str()
        .map(|item| {
            item.strip_prefix("www.")
                .unwrap_or(item)
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let path = parsed.path().to_ascii_lowercase();
    if (host == "youtube.com"
        && (matches!(path.as_str(), "/watch" | "/shorts" | "/live")
            || path.starts_with("/watch/")
            || path.starts_with("/shorts/")))
        || host == "youtu.be"
    {
        return GuiStreamTargetKind::ExtractorPageUrl;
    }
    if browser_media_url_path_looks_direct(&path) {
        return GuiStreamTargetKind::DirectMediaUrl;
    }
    GuiStreamTargetKind::DirectMediaUrl
}

pub(in crate::app) fn playlist_entries_from_multiline_text(value: &str) -> Vec<String> {
    value.lines().filter_map(normalized_editable_text).collect()
}

pub(in crate::app) fn playlist_entries_multiline_text(entries: &[String]) -> String {
    entries.join("\n")
}

pub(in crate::app) fn load_playlist_entries_from_path(path: &str) -> Result<Vec<String>, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("Failed to read playlist file '{path}': {error}"))
        .map(|contents| {
            contents
                .lines()
                .filter_map(normalized_editable_text)
                .collect()
        })
}

pub(in crate::app) fn save_playlist_entries_to_path(
    path: &str,
    entries: &[String],
) -> Result<(), String> {
    std::fs::write(path, playlist_entries_multiline_text(entries))
        .map_err(|error| format!("Failed to save playlist file '{path}': {error}"))
}

pub(in crate::app) fn playlist_next_shuffle_state(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state
}

pub(in crate::app) fn shuffle_playlist_entries_in_place(entries: &mut [String], seed: u64) {
    if entries.len() <= 1 {
        return;
    }
    let mut state = seed;
    for index in (1..entries.len()).rev() {
        let random_value = playlist_next_shuffle_state(&mut state);
        let swap_index = (random_value as usize) % (index + 1);
        entries.swap(index, swap_index);
    }
}

pub(in crate::app) fn browser_format_time(seconds: f64) -> String {
    let rounded = seconds.abs().round() as i64;
    let sign = if seconds.is_sign_negative() { "-" } else { "" };
    let days = rounded / 86_400;
    let hours = (rounded % 86_400) / 3_600;
    let minutes = (rounded % 3_600) / 60;
    let seconds = rounded % 60;
    if days > 0 {
        format!("{sign}{days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{minutes:02}:{seconds:02}")
    }
}

pub(in crate::app) fn browser_format_duration_label(value: Option<f64>) -> String {
    let Some(seconds) = value else {
        return String::new();
    };
    format!("({})", browser_format_time(seconds))
}

pub(in crate::app) fn browser_format_size_label(value: Option<&FileSize>) -> String {
    let Some(bytes) = value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.display_value().parse::<f64>().ok())
    }) else {
        return String::new();
    };
    if bytes <= 0.0 {
        return "???".to_owned();
    }
    let megabytes = (bytes / 1_048_576.0).floor() as i64;
    format!("{megabytes} MB")
}
