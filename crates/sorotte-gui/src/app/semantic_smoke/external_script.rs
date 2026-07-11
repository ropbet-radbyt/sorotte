use super::super::{StoredClientSettingsMvp, semantic_driver::GuiSemanticScenario};
use super::{GuiSemanticScenarioReport, run_gui_semantic_scenario};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ExternalSemanticScenarioMetadata {
    scenario_name: Option<String>,
    expected_view: Option<String>,
    expected_modal: Option<String>,
    expected_pending: Option<String>,
}

fn parse_external_metadata_text(key: &str, token: &str) -> Result<String, String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        Err(format!("semantic script metadata {key} cannot be empty"))
    } else {
        Ok(trimmed.to_owned())
    }
}

fn apply_external_semantic_metadata_line(
    metadata: &mut ExternalSemanticScenarioMetadata,
    line: &str,
) -> Result<bool, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(false);
    }

    let mut fields = trimmed.split('\t');
    let Some(command) = fields.next() else {
        return Ok(false);
    };
    if command != "meta" {
        return Ok(false);
    }

    let key = fields
        .next()
        .ok_or_else(|| "meta requires a key".to_owned())?;
    let value = fields
        .next()
        .ok_or_else(|| format!("meta {key} requires a value"))?;
    if fields.next().is_some() {
        return Err(format!("meta {key} accepts exactly one value"));
    }

    let value = parse_external_metadata_text(key, value)?;
    match key {
        "name" => metadata.scenario_name = Some(value),
        "expect-view" => metadata.expected_view = Some(value),
        "expect-modal" => metadata.expected_modal = Some(value),
        "expect-pending" => metadata.expected_pending = Some(value),
        _ => return Err(format!("unknown semantic script metadata key {key:?}")),
    }
    Ok(true)
}

fn parse_external_setting_bool(token: &str) -> Result<bool, String> {
    match token {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "semantic script setting boolean must be 'true' or 'false', got {token:?}"
        )),
    }
}

fn parse_external_optional_text(token: &str) -> Option<String> {
    match token {
        "<none>" => None,
        _ => Some(token.to_owned()),
    }
}

fn push_external_public_server(settings: &mut StoredClientSettingsMvp, label: &str, address: &str) {
    settings
        .public_servers
        .get_or_insert_with(Vec::new)
        .push((label.to_owned(), address.to_owned()));
}

fn push_external_media_search_directory(settings: &mut StoredClientSettingsMvp, path: &str) {
    settings
        .media_search_directories
        .get_or_insert_with(Vec::new)
        .push(path.to_owned());
}

fn apply_external_semantic_setting_line(
    settings: &mut StoredClientSettingsMvp,
    line: &str,
) -> Result<bool, String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Ok(false);
    }

    let mut fields = trimmed.split('\t');
    let Some(command) = fields.next() else {
        return Ok(false);
    };
    if command != "setting" {
        return Ok(false);
    }

    let key = fields
        .next()
        .ok_or_else(|| "setting requires a key".to_owned())?;
    match key {
        "host" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting host requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting host accepts exactly one value".to_owned());
            }
            settings.host = parse_external_optional_text(value);
        }
        "port" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting port requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting port accepts exactly one value".to_owned());
            }
            settings.port = match value {
                "<none>" => None,
                _ => Some(value.parse::<u16>().map_err(|_| {
                    format!("setting port must be a valid u16 or <none>, got {value:?}")
                })?),
            };
        }
        "username" | "name" => {
            let value = fields
                .next()
                .ok_or_else(|| format!("setting {key} requires a value"))?;
            if fields.next().is_some() {
                return Err(format!("setting {key} accepts exactly one value"));
            }
            settings.username = parse_external_optional_text(value);
        }
        "room" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting room requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting room accepts exactly one value".to_owned());
            }
            settings.room = parse_external_optional_text(value);
        }
        "server-password" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting server-password requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting server-password accepts exactly one value".to_owned());
            }
            settings.server_password = parse_external_optional_text(value).map(Into::into);
        }
        "player-path" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting player-path requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting player-path accepts exactly one value".to_owned());
            }
            settings.player_path = parse_external_optional_text(value);
        }
        "chat-input-enabled" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting chat-input-enabled requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting chat-input-enabled accepts exactly one value".to_owned());
            }
            settings.chat_input_enabled = Some(parse_external_setting_bool(value)?);
        }
        "chat-output-enabled" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting chat-output-enabled requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting chat-output-enabled accepts exactly one value".to_owned());
            }
            settings.chat_output_enabled = Some(parse_external_setting_bool(value)?);
        }
        "shared-playlist-enabled" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting shared-playlist-enabled requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting shared-playlist-enabled accepts exactly one value".to_owned());
            }
            settings.shared_playlist_enabled = Some(parse_external_setting_bool(value)?);
        }
        "plex-selected-server" => {
            let machine_identifier = fields.next().ok_or_else(|| {
                "setting plex-selected-server requires a machine identifier".to_owned()
            })?;
            let uri = fields
                .next()
                .ok_or_else(|| "setting plex-selected-server requires a URI".to_owned())?;
            let token = fields
                .next()
                .ok_or_else(|| "setting plex-selected-server requires a token".to_owned())?;
            if fields.next().is_some() {
                return Err("setting plex-selected-server accepts exactly three values".to_owned());
            }
            settings.plex_user_token = Some(token.into());
            settings.plex_selected_server_id = Some(machine_identifier.to_owned());
            settings.plex_selected_server_url = Some(uri.to_owned());
            settings.plex_selected_server_token = Some(token.into());
        }
        "plex-sync-enabled" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting plex-sync-enabled requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting plex-sync-enabled accepts exactly one value".to_owned());
            }
            settings.plex_sync_enabled = Some(parse_external_setting_bool(value)?);
        }
        "plex-streaming-enabled" => {
            let value = fields
                .next()
                .ok_or_else(|| "setting plex-streaming-enabled requires a value".to_owned())?;
            if fields.next().is_some() {
                return Err("setting plex-streaming-enabled accepts exactly one value".to_owned());
            }
            settings.plex_streaming_enabled = Some(parse_external_setting_bool(value)?);
        }
        "public-server" => {
            let label = fields
                .next()
                .ok_or_else(|| "setting public-server requires a label".to_owned())?;
            let address = fields
                .next()
                .ok_or_else(|| "setting public-server requires an address".to_owned())?;
            if fields.next().is_some() {
                return Err("setting public-server accepts exactly two values".to_owned());
            }
            push_external_public_server(settings, label, address);
        }
        "media-search-directory" => {
            let path = fields
                .next()
                .ok_or_else(|| "setting media-search-directory requires a path".to_owned())?;
            if fields.next().is_some() {
                return Err("setting media-search-directory accepts exactly one value".to_owned());
            }
            push_external_media_search_directory(settings, path);
        }
        _ => return Err(format!("unknown semantic script setting key {key:?}")),
    }
    Ok(true)
}

pub(super) fn parse_external_semantic_script(
    script: &str,
) -> Result<
    (
        ExternalSemanticScenarioMetadata,
        StoredClientSettingsMvp,
        String,
    ),
    String,
> {
    let mut metadata = ExternalSemanticScenarioMetadata::default();
    let mut settings = StoredClientSettingsMvp::default();
    let mut step_lines = Vec::new();
    for (line_index, line) in script.lines().enumerate() {
        let consumed = match apply_external_semantic_metadata_line(&mut metadata, line) {
            Ok(consumed) => consumed,
            Err(error) => {
                return Err(format!(
                    "semantic script line {} failed: {error}",
                    line_index + 1
                ));
            }
        };
        if consumed {
            step_lines.push(String::new());
            continue;
        }
        match apply_external_semantic_setting_line(&mut settings, line) {
            Ok(true) => step_lines.push(String::new()),
            Ok(false) => step_lines.push(line.to_owned()),
            Err(error) => {
                return Err(format!(
                    "semantic script line {} failed: {error}",
                    line_index + 1
                ));
            }
        }
    }
    Ok((metadata, settings, step_lines.join("\n")))
}

pub(super) fn run_gui_semantic_external_script(
    script_source_label: &str,
    script: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    let (metadata, initial_settings, step_script) = parse_external_semantic_script(script)?;
    let scenario =
        GuiSemanticScenario::from_script("external-script", initial_settings, &step_script)
            .map_err(|error| {
                format!("failed to parse semantic scenario script {script_source_label}: {error}")
            })?;
    let mut report = run_gui_semantic_scenario(scenario)?;
    if let Some(expected_view) = metadata.expected_view.as_deref()
        && report.view != expected_view
    {
        return Err(format!(
            "semantic scenario script {script_source_label} expected final view {expected_view:?}, got {:?}",
            report.view
        ));
    }
    if let Some(expected_modal) = metadata.expected_modal.as_deref()
        && report.modal != expected_modal
    {
        return Err(format!(
            "semantic scenario script {script_source_label} expected final modal {expected_modal:?}, got {:?}",
            report.modal
        ));
    }
    if let Some(expected_pending) = metadata.expected_pending.as_deref()
        && report.pending != expected_pending
    {
        return Err(format!(
            "semantic scenario script {script_source_label} expected final pending {expected_pending:?}, got {:?}",
            report.pending
        ));
    }
    if let Some(scenario_name) = metadata.scenario_name {
        report.scenario = scenario_name;
    }
    Ok(report)
}
