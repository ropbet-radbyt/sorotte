use super::*;

pub(crate) fn legacy_stats_snapshot_start_delay_seconds_for_port(port: u16) -> f64 {
    SERVER_STATS_DELAY_STEP_SECONDS * (f64::from(port % 10) + 1.0)
}

pub(crate) fn parse_numeric_version_components(version: &str) -> Option<Vec<u32>> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut components = Vec::new();
    for part in trimmed.split('.') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        components.push(part.parse().ok()?);
    }

    Some(components)
}

pub(crate) fn is_client_version_outdated(client_version: &str, server_version: &str) -> bool {
    let Some(mut client_components) = parse_numeric_version_components(client_version) else {
        return false;
    };
    let Some(mut server_components) = parse_numeric_version_components(server_version) else {
        return false;
    };

    let width = client_components.len().max(server_components.len());
    client_components.resize(width, 0);
    server_components.resize(width, 0);
    client_components < server_components
}

pub(crate) fn client_version_meets_minimum(client_version: &str, minimum_version: &str) -> bool {
    let Some(mut client_components) = parse_numeric_version_components(client_version) else {
        return false;
    };
    let Some(mut minimum_components) = parse_numeric_version_components(minimum_version) else {
        return false;
    };

    let width = client_components.len().max(minimum_components.len());
    client_components.resize(width, 0);
    minimum_components.resize(width, 0);
    client_components >= minimum_components
}

pub(crate) fn render_motd_template(template: &str, client_version: &str) -> String {
    template
        .replace("{client_version}", client_version)
        .replace("{latest_version}", LEGACY_COMPAT_SERVER_VERSION)
        .replace("{upgrade_url}", LEGACY_COMPAT_UPGRADE_URL)
}

fn render_python_dollar_motd_template(
    template: &str,
    user_ip: &str,
    username: &str,
    room_name: &str,
) -> Result<String, ()> {
    let mut rendered = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            rendered.push(ch);
            continue;
        }

        match chars.peek().copied() {
            Some('$') => {
                chars.next();
                rendered.push('$');
            }
            Some('{') => {
                chars.next();
                let mut name = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    name.push(next);
                }
                if !closed {
                    return Err(());
                }
                rendered.push_str(motd_template_variable(&name, user_ip, username, room_name)?);
            }
            Some(next) if next == '_' || next.is_ascii_alphabetic() => {
                let mut name = String::new();
                while let Some(next) = chars.peek().copied() {
                    if next == '_' || next.is_ascii_alphanumeric() {
                        name.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                rendered.push_str(motd_template_variable(&name, user_ip, username, room_name)?);
            }
            _ => return Err(()),
        }
    }
    Ok(rendered)
}

fn motd_template_variable<'a>(
    name: &str,
    user_ip: &'a str,
    username: &'a str,
    room_name: &'a str,
) -> Result<&'a str, ()> {
    match name {
        "version" => Ok(LEGACY_COMPAT_SERVER_VERSION),
        "userIp" => Ok(user_ip),
        "username" => Ok(username),
        "room" => Ok(room_name),
        _ => Err(()),
    }
}

fn render_custom_motd_template(
    template: &str,
    client_version: &str,
    user_ip: &str,
    username: &str,
    room_name: &str,
) -> Result<String, ()> {
    let rendered = render_python_dollar_motd_template(template, user_ip, username, room_name)?;
    Ok(rendered
        .replace("{client_version}", client_version)
        .replace("{latest_version}", LEGACY_COMPAT_SERVER_VERSION)
        .replace("{upgrade_url}", LEGACY_COMPAT_UPGRADE_URL)
        .replace("{version}", LEGACY_COMPAT_SERVER_VERSION)
        .replace("{userIp}", user_ip)
        .replace("{username}", username)
        .replace("{room}", room_name))
}

fn motd_too_long_message(actual_chars: usize) -> String {
    format!(
        "{LEGACY_SERVER_MOTD_TOO_LONG_PREFIX} {LEGACY_SERVER_MAX_TEMPLATE_LENGTH} chars, {actual_chars} given."
    )
}

pub(crate) fn default_motd_for_client_version(client_version: &str) -> String {
    if is_client_version_outdated(client_version, LEGACY_COMPAT_SERVER_VERSION) {
        return render_motd_template(DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version);
    }
    String::new()
}

#[cfg(test)]
pub(crate) fn motd_for_client_version(
    client_version: &str,
    motd_template_override: Option<&str>,
) -> String {
    motd_for_client_context(client_version, motd_template_override, "", "", "")
}

pub(crate) fn motd_for_client_context(
    client_version: &str,
    motd_template_override: Option<&str>,
    user_ip: &str,
    username: &str,
    room_name: &str,
) -> String {
    let is_outdated = is_client_version_outdated(client_version, LEGACY_COMPAT_SERVER_VERSION);
    if let Some(template) = motd_template_override {
        if template.trim().is_empty() {
            return String::new();
        }
        let custom_motd = match render_custom_motd_template(
            template,
            client_version,
            user_ip,
            username,
            room_name,
        ) {
            Ok(custom_motd) => custom_motd,
            Err(()) => return LEGACY_SERVER_MOTD_UNESCAPED_PLACEHOLDERS.to_owned(),
        };
        let motd = if is_outdated {
            let warning_motd = render_motd_template(DEFAULT_OUTDATED_MOTD_TEMPLATE, client_version);
            format!("{warning_motd}\n{custom_motd}")
        } else {
            custom_motd
        };
        if motd.chars().count() <= LEGACY_SERVER_MAX_TEMPLATE_LENGTH {
            return motd;
        }
        return motd_too_long_message(motd.chars().count());
    }
    default_motd_for_client_version(client_version)
}

pub(crate) fn truncate_text_to_max_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn playlist_is_valid(files: &[String]) -> bool {
    if files.len() > DEFAULT_PLAYLIST_MAX_ITEMS {
        return false;
    }
    files.iter().map(|file| file.chars().count()).sum::<usize>() <= DEFAULT_PLAYLIST_MAX_CHARACTERS
}

pub(crate) fn legacy_server_password_token_md5_hex(token: &str) -> String {
    lowercase_hex(Md5::digest(token.as_bytes()))
}

pub(crate) fn server_password_token_matches_legacy_compatible(
    presented_token: &str,
    configured_token: &str,
) -> bool {
    // Accept raw tokens for Rust-Rust interoperability and legacy-Python MD5 tokens for parity.
    presented_token == configured_token
        || presented_token == legacy_server_password_token_md5_hex(configured_token)
}

pub(crate) fn persistent_rooms_notice_motd(
    base_motd: String,
    persistent_rooms_enabled: bool,
    client_supports_persistent_rooms: bool,
) -> String {
    if !persistent_rooms_enabled || client_supports_persistent_rooms {
        return base_motd;
    }
    if base_motd.is_empty() {
        return LEGACY_PERSISTENT_ROOMS_NOTICE.to_owned();
    }
    format!("{LEGACY_PERSISTENT_ROOMS_NOTICE}\n\n{base_motd}")
}

pub(crate) fn room_name_is_marked_temporary(room_name: &str) -> bool {
    let room_name = room_name.to_ascii_lowercase();
    room_name.ends_with("-temp") || room_name.contains("-temp:")
}

pub(crate) fn playlist_as_multiline(files: &[String]) -> String {
    files.join("\n")
}

pub(crate) fn multiline_as_playlist(multiline: &str) -> Vec<String> {
    if multiline.is_empty() {
        return Vec::new();
    }
    multiline.split('\n').map(str::to_owned).collect()
}

pub(crate) fn parse_permanent_rooms_file(contents: &str) -> BTreeSet<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

pub(crate) fn legacy_dummy_list_entry() -> ListUserEntry {
    ListUserEntry::new()
        .with_position(0.0)
        .with_file(json!({}))
        .with_controller(false)
        .with_is_ready(true)
        .with_features(json!([]))
}
