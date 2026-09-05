use std::collections::{BTreeMap, BTreeSet};

use crate::legacy_settings::StoredClientSettingsMvp;

use super::{
    helpers::{
        ini_section_name, remove_ini_value_legacy_compatible,
        unescape_sorotte_ini_value_legacy_compatible, upsert_ini_value_legacy_compatible,
    },
    writer::upsert_sorotte_ini_stored_client_settings_mvp,
};

fn recognized_values(settings: &StoredClientSettingsMvp) -> BTreeMap<(String, String), String> {
    let rendered = upsert_sorotte_ini_stored_client_settings_mvp("", settings);
    let mut section = String::new();
    let mut values = BTreeMap::new();
    for line in rendered.lines() {
        if let Some(name) = ini_section_name(line.trim()) {
            section = name.to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            values.insert(
                (section.clone(), key.trim().to_owned()),
                unescape_sorotte_ini_value_legacy_compatible(value.trim()),
            );
        }
    }
    values
}

/// Apply only intended changes. A field that is unchanged from the caller's
/// baseline must never overwrite a newer value (especially a cleared secret).
pub(super) fn merge_settings_contents(
    contents: &str,
    baseline: &StoredClientSettingsMvp,
    desired: &StoredClientSettingsMvp,
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
    for (section, key) in keys {
        if before.get(&(section.clone(), key.clone())) == after.get(&(section.clone(), key.clone()))
        {
            continue;
        }
        changed = true;
        match after.get(&(section.clone(), key.clone())) {
            Some(value) => upsert_ini_value_legacy_compatible(&mut lines, section, key, value),
            None => remove_ini_value_legacy_compatible(&mut lines, section, key),
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
