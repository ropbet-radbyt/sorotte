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
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' => escaped.push_str("%%"),
            character if character.is_control() => {
                escaped.push_str(&format!("%{{{:X}}}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

pub(super) fn unescape_sorotte_ini_value_legacy_compatible(value: &str) -> String {
    let mut unescaped = String::with_capacity(value.len());
    let mut characters = value.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if character != '%' {
            unescaped.push(character);
            continue;
        }

        if characters.peek().is_some_and(|(_, next)| *next == '%') {
            characters.next();
            unescaped.push('%');
            continue;
        }

        let encoded = &value[index..];
        let Some(encoded_body) = encoded.strip_prefix("%{") else {
            unescaped.push('%');
            continue;
        };
        let Some(encoded_end) = encoded_body.find('}') else {
            unescaped.push('%');
            continue;
        };
        let hex = &encoded_body[..encoded_end];
        let decoded = u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .filter(|character| character.is_control());
        let Some(decoded) = decoded else {
            unescaped.push('%');
            continue;
        };
        unescaped.push(decoded);

        let consumed_end = index + 2 + encoded_end + 1;
        while characters
            .peek()
            .is_some_and(|(next_index, _)| *next_index < consumed_end)
        {
            characters.next();
        }
    }
    unescaped
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
    let rendered = format!(
        "{key} = {}",
        escape_sorotte_ini_value_legacy_compatible(value)
    );

    let mut in_section = false;
    let mut insert_at = None;
    let mut found_key = false;
    for (idx, line) in lines.iter_mut().enumerate() {
        let trimmed = line.trim();
        if let Some(candidate) = ini_section_name(trimmed) {
            in_section = candidate.eq_ignore_ascii_case(section);
        } else if in_section
            && let Some((candidate_key, _)) = trimmed.split_once('=')
            && candidate_key.trim().eq_ignore_ascii_case(key)
        {
            // The parser consumes every occurrence, including repeated sections.
            // Rewrite all copies so clearing a secret cannot leave an older copy.
            *line = rendered.clone();
            found_key = true;
        }
        if in_section {
            insert_at = Some(idx + 1);
        }
    }
    if found_key {
        return;
    }
    if let Some(insert_at) = insert_at {
        lines.insert(insert_at, rendered);
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
    let mut in_section = false;
    lines.retain(|line| {
        let trimmed = line.trim();
        if let Some(candidate) = ini_section_name(trimmed) {
            in_section = candidate.eq_ignore_ascii_case(section);
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

pub(super) fn ini_section_name(line: &str) -> Option<&str> {
    line.strip_prefix('[')?.strip_suffix(']').map(str::trim)
}
