use std::collections::{BTreeMap, BTreeSet};

use crate::stored_settings::StoredClientSettings;

use super::{
    fields::INI_FIELDS,
    helpers::{
        escape_sorotte_ini_value, remove_ini_value, unescape_sorotte_ini_value, upsert_ini_value,
    },
};

fn recognized_values(
    settings: &StoredClientSettings,
) -> BTreeMap<(&'static str, &'static str), String> {
    INI_FIELDS
        .iter()
        .filter_map(|field| {
            let value = (field.write)(settings)?;
            // Match the INI read boundary: ordinary outer spaces are trimmed,
            // while escaped control characters remain part of the value.
            let value = unescape_sorotte_ini_value(escape_sorotte_ini_value(&value).trim());
            Some(((field.section, field.key), value))
        })
        .collect()
}

/// Apply only intended changes. A field that is unchanged from the caller's
/// baseline must never overwrite a newer value (especially a cleared secret).
pub(super) fn merge_settings_contents(
    contents: &str,
    baseline: &StoredClientSettings,
    desired: &StoredClientSettings,
) -> String {
    let before = recognized_values(baseline);
    let after = recognized_values(desired);
    let mut lines: Vec<String> = contents
        .strip_prefix('\u{feff}')
        .unwrap_or(contents)
        .lines()
        .map(ToOwned::to_owned)
        .collect();
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    let mut changed = false;
    for key in keys {
        if before.get(key) == after.get(key) {
            continue;
        }
        changed = true;
        let (section, name) = key;
        match after.get(key) {
            Some(value) => upsert_ini_value(&mut lines, section, name, value),
            None => remove_ini_value(&mut lines, section, name),
        }
    }
    if !changed {
        return contents.to_owned();
    }
    let mut rendered = lines.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    if contents.starts_with('\u{feff}') {
        rendered.insert(0, '\u{feff}');
    }
    rendered
}
