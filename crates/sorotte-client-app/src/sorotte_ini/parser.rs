use crate::stored_settings::StoredClientSettings;

use super::{fields::INI_FIELDS, helpers::unescape_sorotte_ini_value};

pub fn parse_sorotte_ini_stored_client_settings(contents: &str) -> StoredClientSettings {
    let mut settings = StoredClientSettings::default();
    let mut current_section: Option<String> = None;
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(section_name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current_section = Some(section_name.trim().to_ascii_lowercase());
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = unescape_sorotte_ini_value(raw_value.trim());
        if let Some(field) = INI_FIELDS.iter().find(|field| {
            current_section.as_deref() == Some(field.section) && field.key.eq_ignore_ascii_case(key)
        }) {
            (field.read)(&mut settings, &value);
        }
    }
    settings
}
