use std::path::Path;

use anyhow::anyhow;

use crate::legacy_settings::StoredClientSettingsMvp;

use super::parser::parse_syncplay_ini_stored_client_settings_mvp;
use super::writer::upsert_syncplay_ini_stored_client_settings_mvp;

pub fn load_syncplay_ini_stored_client_settings_mvp_from_path(
    path: &Path,
) -> anyhow::Result<Option<StoredClientSettingsMvp>> {
    if !path.is_file() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("failed reading stored settings {}: {error}", path.display()))?;
    Ok(Some(parse_syncplay_ini_stored_client_settings_mvp(
        &contents,
    )))
}

pub fn upsert_syncplay_ini_stored_client_settings_mvp_at_path(
    path: &Path,
    settings: &StoredClientSettingsMvp,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            anyhow!(
                "failed creating stored settings directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let existing_contents = if path.is_file() {
        std::fs::read_to_string(path).map_err(|error| {
            anyhow!("failed reading stored settings {}: {error}", path.display())
        })?
    } else {
        String::new()
    };
    let updated_contents =
        upsert_syncplay_ini_stored_client_settings_mvp(&existing_contents, settings);
    std::fs::write(path, updated_contents)
        .map_err(|error| anyhow!("failed writing stored settings {}: {error}", path.display()))
}

pub fn update_syncplay_ini_stored_client_settings_mvp_at_path<F>(
    path: &Path,
    update: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&mut StoredClientSettingsMvp),
{
    let mut settings =
        load_syncplay_ini_stored_client_settings_mvp_from_path(path)?.unwrap_or_default();
    update(&mut settings);
    upsert_syncplay_ini_stored_client_settings_mvp_at_path(path, &settings)
}

pub fn clear_syncplay_ini_stored_client_settings_mvp_at_path(path: &Path) -> anyhow::Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !path.is_file() {
        return Err(anyhow!(
            "stored settings path is not a file and cannot be cleared: {}",
            path.display()
        ));
    }
    std::fs::remove_file(path).map_err(|error| {
        anyhow!(
            "failed clearing stored settings {}: {error}",
            path.display()
        )
    })?;
    Ok(true)
}
