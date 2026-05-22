pub(super) fn parse_ini_bool_legacy_compatible(value: &str) -> Option<bool> {
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

pub(super) fn parse_ini_port_legacy_compatible(value: &str) -> Option<u16> {
    let port = value.trim().parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

pub(super) fn parse_ini_non_negative_f64_legacy_compatible(value: &str) -> Option<f64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

pub(super) fn escape_sorotte_ini_value_legacy_compatible(value: &str) -> String {
    value.replace('%', "%%")
}

pub(super) fn format_ini_bool_legacy_compatible(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

pub(super) fn format_ini_non_negative_f64_legacy_compatible(value: f64) -> Option<String> {
    (value.is_finite() && value >= 0.0).then(|| value.to_string())
}

pub(super) fn parse_ini_i64_legacy_compatible(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok()
}

pub(super) fn upsert_ini_value_legacy_compatible(
    lines: &mut Vec<String>,
    section: &str,
    key: &str,
    value: &str,
) {
    let section_header = format!("[{section}]");
    let mut section_start = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.trim().eq_ignore_ascii_case(&section_header) {
            section_start = Some(idx);
            break;
        }
    }

    let rendered = format!(
        "{key} = {}",
        escape_sorotte_ini_value_legacy_compatible(value)
    );

    if let Some(section_start_idx) = section_start {
        let mut insert_at = lines.len();
        let mut key_index = None;
        for (idx, line) in lines.iter().enumerate().skip(section_start_idx + 1) {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                insert_at = idx;
                break;
            }
            if let Some((candidate_key, _)) = trimmed.split_once('=')
                && candidate_key.trim().eq_ignore_ascii_case(key)
            {
                key_index = Some(idx);
                break;
            }
        }
        if let Some(idx) = key_index {
            lines[idx] = rendered;
        } else {
            lines.insert(insert_at, rendered);
        }
        return;
    }

    if !lines.is_empty() && !lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.push(String::new());
    }
    lines.push(section_header);
    lines.push(rendered);
}

pub(super) fn remove_ini_value_legacy_compatible(
    lines: &mut Vec<String>,
    section: &str,
    key: &str,
) {
    let section_header = format!("[{section}]");
    let mut in_section = false;
    lines.retain(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed.eq_ignore_ascii_case(&section_header);
            return true;
        }
        if !in_section {
            return true;
        }
        !trimmed
            .split_once('=')
            .is_some_and(|(candidate_key, _)| candidate_key.trim().eq_ignore_ascii_case(key))
    });
}
