use std::collections::BTreeMap;

pub fn parse_serialized_string_list_legacy_compatible(value: &str) -> Option<Vec<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        let inner = inner.trim();
        if inner.is_empty() {
            return Some(Vec::new());
        }
        let values = inner
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| {
                let unquoted = entry
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
                    .or_else(|| entry.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
                    .unwrap_or(entry);
                unquoted
                    .replace("\\\\", "\\")
                    .replace("\\'", "'")
                    .replace("\\\"", "\"")
            })
            .collect::<Vec<_>>();
        return Some(values);
    }
    parse_unbracketed_string_list_fallback_legacy_compatible(trimmed)
}

pub fn format_serialized_string_list_legacy_compatible(values: &[String]) -> String {
    let rendered = values
        .iter()
        .map(|value| {
            let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
            format!("'{escaped}'")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered}]")
}

pub fn parse_serialized_per_player_arguments_map_legacy_compatible(
    value: &str,
) -> Option<BTreeMap<String, Vec<String>>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut index = 0usize;
    skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
    if trimmed.get(index..).and_then(|rest| rest.chars().next())? != '{' {
        return None;
    }
    index += 1;

    let mut parsed = BTreeMap::new();
    loop {
        skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
        let next = trimmed.get(index..).and_then(|rest| rest.chars().next())?;
        if next == '}' {
            index += 1;
            break;
        }
        let key = parse_serialized_python_string_cursor_legacy_compatible(trimmed, &mut index)?;
        skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
        if trimmed.get(index..).and_then(|rest| rest.chars().next())? != ':' {
            return None;
        }
        index += 1;
        let args =
            parse_serialized_python_string_list_cursor_legacy_compatible(trimmed, &mut index)?;
        parsed.insert(key, args);
        skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
        let delim = trimmed.get(index..).and_then(|rest| rest.chars().next())?;
        if delim == ',' {
            index += 1;
            continue;
        }
        if delim == '}' {
            index += 1;
            break;
        }
        return None;
    }
    skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
    if index != trimmed.len() {
        return None;
    }
    Some(parsed)
}

pub fn format_serialized_per_player_arguments_map_legacy_compatible(
    values: &BTreeMap<String, Vec<String>>,
) -> String {
    let rendered = values
        .iter()
        .map(|(player_path, args)| {
            format!(
                "{}: {}",
                format_serialized_python_string_legacy_compatible(player_path),
                format_serialized_string_list_legacy_compatible(args)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{rendered}}}")
}

pub fn parse_serialized_public_servers_list_legacy_compatible(
    value: &str,
) -> Option<Vec<(String, String)>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut index = 0usize;
    skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
    if trimmed.get(index..).and_then(|rest| rest.chars().next())? != '[' {
        return None;
    }
    index += 1;

    let mut parsed = Vec::new();
    loop {
        skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
        let next = trimmed.get(index..).and_then(|rest| rest.chars().next())?;
        if next == ']' {
            index += 1;
            break;
        }
        parsed.push(
            parse_serialized_python_string_pair_cursor_legacy_compatible(trimmed, &mut index)?,
        );
        skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
        let delim = trimmed.get(index..).and_then(|rest| rest.chars().next())?;
        if delim == ',' {
            index += 1;
            continue;
        }
        if delim == ']' {
            index += 1;
            break;
        }
        return None;
    }

    skip_serialized_python_whitespace_cursor_legacy_compatible(trimmed, &mut index);
    if index != trimmed.len() {
        return None;
    }
    Some(parsed)
}

pub fn format_serialized_public_servers_list_legacy_compatible(
    values: &[(String, String)],
) -> String {
    let rendered = values
        .iter()
        .map(|(label, address)| {
            format!(
                "[{}, {}]",
                format_serialized_python_string_legacy_compatible(label),
                format_serialized_python_string_legacy_compatible(address)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered}]")
}

fn parse_unbracketed_string_list_fallback_legacy_compatible(value: &str) -> Option<Vec<String>> {
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

fn format_serialized_python_string_legacy_compatible(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

fn skip_serialized_python_whitespace_cursor_legacy_compatible(input: &str, index: &mut usize) {
    while let Some(ch) = input.get(*index..).and_then(|rest| rest.chars().next()) {
        if !ch.is_whitespace() {
            break;
        }
        *index += ch.len_utf8();
    }
}

fn parse_serialized_python_string_cursor_legacy_compatible(
    input: &str,
    index: &mut usize,
) -> Option<String> {
    let quote = input.get(*index..).and_then(|rest| rest.chars().next())?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    *index += quote.len_utf8();

    let mut parsed = String::new();
    while let Some(ch) = input.get(*index..).and_then(|rest| rest.chars().next()) {
        *index += ch.len_utf8();
        if ch == quote {
            return Some(parsed);
        }
        if ch == '\\' {
            let escaped = input.get(*index..).and_then(|rest| rest.chars().next())?;
            *index += escaped.len_utf8();
            match escaped {
                '\\' => parsed.push('\\'),
                '\'' => parsed.push('\''),
                '"' => parsed.push('"'),
                'n' => parsed.push('\n'),
                'r' => parsed.push('\r'),
                't' => parsed.push('\t'),
                other => parsed.push(other),
            }
            continue;
        }
        parsed.push(ch);
    }
    None
}

fn parse_serialized_python_string_list_cursor_legacy_compatible(
    input: &str,
    index: &mut usize,
) -> Option<Vec<String>> {
    skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
    let ch = input.get(*index..).and_then(|rest| rest.chars().next())?;
    if ch != '[' {
        return None;
    }
    *index += 1;
    let mut values = Vec::new();
    loop {
        skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
        let next = input.get(*index..).and_then(|rest| rest.chars().next())?;
        if next == ']' {
            *index += 1;
            return Some(values);
        }
        let value = parse_serialized_python_string_cursor_legacy_compatible(input, index)?;
        values.push(value);
        skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
        let delim = input.get(*index..).and_then(|rest| rest.chars().next())?;
        if delim == ',' {
            *index += 1;
            continue;
        }
        if delim == ']' {
            *index += 1;
            return Some(values);
        }
        return None;
    }
}

fn parse_serialized_python_string_pair_cursor_legacy_compatible(
    input: &str,
    index: &mut usize,
) -> Option<(String, String)> {
    skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
    let open = input.get(*index..).and_then(|rest| rest.chars().next())?;
    let close = match open {
        '[' => ']',
        '(' => ')',
        _ => return None,
    };
    *index += open.len_utf8();

    skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
    let label = parse_serialized_python_string_cursor_legacy_compatible(input, index)?;
    skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
    if input.get(*index..).and_then(|rest| rest.chars().next())? != ',' {
        return None;
    }
    *index += 1;

    skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
    let address = parse_serialized_python_string_cursor_legacy_compatible(input, index)?;
    skip_serialized_python_whitespace_cursor_legacy_compatible(input, index);
    if input.get(*index..).and_then(|rest| rest.chars().next())? != close {
        return None;
    }
    *index += close.len_utf8();

    Some((label, address))
}

#[cfg(test)]
mod tests {
    use super::{
        format_serialized_per_player_arguments_map_legacy_compatible,
        format_serialized_public_servers_list_legacy_compatible,
        format_serialized_string_list_legacy_compatible,
        parse_serialized_per_player_arguments_map_legacy_compatible,
        parse_serialized_public_servers_list_legacy_compatible,
        parse_serialized_string_list_legacy_compatible,
    };

    #[test]
    fn serialized_string_list_parses_bracketed_and_fallback_forms() {
        assert_eq!(
            parse_serialized_string_list_legacy_compatible("['one', 'two']"),
            Some(vec!["one".to_owned(), "two".to_owned()])
        );
        assert_eq!(
            parse_serialized_string_list_legacy_compatible("one; two"),
            Some(vec!["one".to_owned(), "two".to_owned()])
        );
        assert_eq!(
            format_serialized_string_list_legacy_compatible(&["one".to_owned(), "two".to_owned()]),
            "['one', 'two']"
        );
    }

    #[test]
    fn per_player_arguments_roundtrip() {
        let raw = r#"{'C:\\mpv\\mpv.exe': ['--profile=fast', '--no-border'], 'mpv': ['--fs']}"#;
        let parsed = parse_serialized_per_player_arguments_map_legacy_compatible(raw)
            .expect("expected per-player arguments map");
        let rendered = format_serialized_per_player_arguments_map_legacy_compatible(&parsed);
        let reparsed = parse_serialized_per_player_arguments_map_legacy_compatible(&rendered)
            .expect("expected reparsed map");

        assert_eq!(parsed, reparsed);
        assert!(parsed.contains_key("C:\\mpv\\mpv.exe"));
        assert!(parsed.contains_key("mpv"));
    }

    #[test]
    fn public_servers_roundtrip() {
        let raw = r#"[['Official', 'syncplay.pl:8999'], ('Local', '127.0.0.1:8995')]"#;
        let parsed = parse_serialized_public_servers_list_legacy_compatible(raw)
            .expect("expected public servers list");
        let rendered = format_serialized_public_servers_list_legacy_compatible(&parsed);
        let reparsed = parse_serialized_public_servers_list_legacy_compatible(&rendered)
            .expect("expected reparsed public servers");

        assert_eq!(parsed, reparsed);
        assert_eq!(parsed.len(), 2);
    }
}
