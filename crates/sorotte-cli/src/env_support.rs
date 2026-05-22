use sorotte_client_core::PrivacyMode;

pub(super) fn env_trimmed(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

pub(super) fn env_flag_enabled(name: &str) -> bool {
    env_trimmed(name)
        .and_then(|value| parse_env_bool_legacy_compatible(&value))
        .unwrap_or(false)
}

pub(super) fn env_flag_override(name: &str) -> Option<bool> {
    env_trimmed(name).and_then(|value| parse_env_bool_legacy_compatible(&value))
}

pub(super) fn parse_env_bool_legacy_compatible(value: &str) -> Option<bool> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized == "1"
        || normalized.eq_ignore_ascii_case("true")
        || normalized.eq_ignore_ascii_case("yes")
        || normalized.eq_ignore_ascii_case("on")
    {
        return Some(true);
    }
    if normalized == "0"
        || normalized.eq_ignore_ascii_case("false")
        || normalized.eq_ignore_ascii_case("no")
        || normalized.eq_ignore_ascii_case("off")
    {
        return Some(false);
    }
    None
}

pub(super) fn parse_env_port_legacy_compatible(value: &str) -> Option<u16> {
    let port = value.trim().parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

pub(super) fn env_port(name: &str) -> Option<u16> {
    env_trimmed(name).and_then(|value| parse_env_port_legacy_compatible(&value))
}

pub(super) fn parse_env_string_list_legacy_compatible(value: &str) -> Option<Vec<String>> {
    let values: Vec<String> = value
        .split([',', ';', '\n', '\r'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

pub(super) fn env_string_list(name: &str) -> Option<Vec<String>> {
    env_trimmed(name).and_then(|value| parse_env_string_list_legacy_compatible(&value))
}

pub(super) fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

pub(super) fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

pub(super) fn parse_env_non_negative_f64_legacy_compatible(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

pub(super) fn env_non_negative_f64(name: &str) -> Option<f64> {
    env_trimmed(name).and_then(|value| parse_env_non_negative_f64_legacy_compatible(&value))
}

pub(super) fn env_privacy_mode(name: &str) -> Option<PrivacyMode> {
    env_trimmed(name).and_then(|value| PrivacyMode::from_legacy_name(value.as_str()))
}
