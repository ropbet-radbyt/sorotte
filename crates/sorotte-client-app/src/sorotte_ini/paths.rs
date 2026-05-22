use std::path::Path;

use anyhow::anyhow;

use crate::legacy_settings::StoredClientSettingsMvp;

use super::parser::parse_sorotte_ini_stored_client_settings_mvp;
use super::writer::{
    upsert_sorotte_ini_stored_client_settings_mvp,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
};

pub fn load_sorotte_ini_stored_client_settings_mvp_from_path(
    path: &Path,
) -> anyhow::Result<Option<StoredClientSettingsMvp>> {
    if !path.is_file() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| anyhow!("failed reading stored settings {}: {error}", path.display()))?;
    Ok(Some(parse_sorotte_ini_stored_client_settings_mvp(
        &contents,
    )))
}

pub fn upsert_sorotte_ini_stored_client_settings_mvp_at_path(
    path: &Path,
    settings: &StoredClientSettingsMvp,
) -> anyhow::Result<()> {
    upsert_sorotte_ini_stored_client_settings_mvp_at_path_with_writer(
        path,
        settings,
        upsert_sorotte_ini_stored_client_settings_mvp,
    )
}

pub fn upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity_at_path(
    path: &Path,
    settings: &StoredClientSettingsMvp,
) -> anyhow::Result<()> {
    upsert_sorotte_ini_stored_client_settings_mvp_at_path_with_writer(
        path,
        settings,
        upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
    )
}

fn upsert_sorotte_ini_stored_client_settings_mvp_at_path_with_writer(
    path: &Path,
    settings: &StoredClientSettingsMvp,
    writer: fn(&str, &StoredClientSettingsMvp) -> String,
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
    let updated_contents = writer(&existing_contents, settings);
    std::fs::write(path, updated_contents)
        .map_err(|error| anyhow!("failed writing stored settings {}: {error}", path.display()))
}

pub fn update_sorotte_ini_stored_client_settings_mvp_at_path<F>(
    path: &Path,
    update: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&mut StoredClientSettingsMvp),
{
    let mut settings =
        load_sorotte_ini_stored_client_settings_mvp_from_path(path)?.unwrap_or_default();
    update(&mut settings);
    upsert_sorotte_ini_stored_client_settings_mvp_at_path(path, &settings)
}

pub fn clear_sorotte_ini_stored_client_settings_mvp_at_path(path: &Path) -> anyhow::Result<bool> {
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
