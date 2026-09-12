use crate::stored_settings::StoredClientSettings;

use super::{
    fields::INI_FIELDS,
    helpers::{remove_ini_value, upsert_ini_value},
};

pub fn upsert_sorotte_ini_stored_client_settings(
    existing_contents: &str,
    settings: &StoredClientSettings,
) -> String {
    upsert_sorotte_ini_stored_client_settings_with_plex_identity_clear(
        existing_contents,
        settings,
        false,
    )
}

pub fn upsert_sorotte_ini_stored_client_settings_clearing_plex_identity(
    existing_contents: &str,
    settings: &StoredClientSettings,
) -> String {
    upsert_sorotte_ini_stored_client_settings_with_plex_identity_clear(
        existing_contents,
        settings,
        true,
    )
}

fn upsert_sorotte_ini_stored_client_settings_with_plex_identity_clear(
    existing_contents: &str,
    settings: &StoredClientSettings,
    clear_plex_identity: bool,
) -> String {
    let had_bom = existing_contents.starts_with('\u{feff}');
    let mut lines = existing_contents
        .strip_prefix('\u{feff}')
        .unwrap_or(existing_contents)
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if clear_plex_identity {
        remove_ini_value(&mut lines, "plex", "userToken");
        remove_ini_value(&mut lines, "plex", "selectedServerId");
        remove_ini_value(&mut lines, "plex", "selectedServerUrl");
        remove_ini_value(&mut lines, "plex", "selectedServerToken");
    }
    for field in INI_FIELDS.iter() {
        if let Some(value) = (field.write)(settings) {
            upsert_ini_value(&mut lines, field.section, field.key, &value);
        }
    }
    let mut output = lines.join("\n");
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    if had_bom {
        format!("\u{feff}{output}")
    } else {
        output
    }
}
