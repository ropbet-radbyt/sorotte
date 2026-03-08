use std::sync::OnceLock;

use super::StoredClientSettingsMvp;
use super::semantic_driver::GuiSemanticScenario;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiSemanticScenarioReport {
    pub scenario: String,
    pub view: String,
    pub modal: String,
    pub pending: String,
    pub widgets: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiSemanticScenarioDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    pub script: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiSemanticOutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiSemanticScenarioSource {
    Named(String),
    ScriptPath(String),
    InlineScript(String),
}

impl GuiSemanticScenarioReport {
    fn render_text(&self) -> String {
        format!(
            "result=ok\nscenario={}\nview={}\nmodal={}\npending={}\nwidgets={}\n",
            self.scenario, self.view, self.modal, self.pending, self.widgets
        )
    }

    fn render_json(&self) -> String {
        format!(
            "{{\"result\":\"ok\",\"scenario\":{},\"view\":{},\"modal\":{},\"pending\":{},\"widgets\":{}}}\n",
            render_json_string(&self.scenario),
            render_json_string(&self.view),
            render_json_string(&self.modal),
            render_json_string(&self.pending),
            self.widgets
        )
    }

    pub fn render(&self, format: GuiSemanticOutputFormat) -> String {
        match format {
            GuiSemanticOutputFormat::Text => self.render_text(),
            GuiSemanticOutputFormat::Json => self.render_json(),
        }
    }
}

fn render_json_string(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => rendered.push_str("\\\\"),
            '"' => rendered.push_str("\\\""),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            _ => rendered.push(ch),
        }
    }
    rendered.push('"');
    rendered
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ExternalSemanticScenarioMetadata {
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
            settings.server_password = parse_external_optional_text(value);
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

fn parse_external_semantic_script(
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

fn run_gui_semantic_scenario(
    scenario: GuiSemanticScenario,
) -> Result<GuiSemanticScenarioReport, String> {
    let driver = scenario.run()?;
    Ok(GuiSemanticScenarioReport {
        scenario: scenario.name().to_owned(),
        view: driver.active_view_label().to_owned(),
        modal: driver.active_modal_label().to_owned(),
        pending: driver.pending_operation_label().to_owned(),
        widgets: driver.widget_count(),
    })
}

const GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_SCRIPT: &str =
    include_str!("semantic_scenarios/configuration-surface-flow.txt");
static GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_DESCRIPTION: &str = "Edits configuration fields, surfaces validation and command availability, saves, then exercises public-server and media-search pending flows.";
const GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_SCRIPT: &str =
    include_str!("semantic_scenarios/core-shell-smoke-flow.txt");
static GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_DESCRIPTION: &str =
    "Ports the non-transport Windows smoke path into a platform-neutral shell scenario.";
const GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT: &str =
    include_str!("semantic_scenarios/runtime-chat-flow.txt");
static GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_DESCRIPTION: &str =
    "Applies runtime session state, verifies playlist projection, and completes a local chat send.";
const GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT: &str =
    include_str!("semantic_scenarios/runtime-transport-churn-flow.txt");
static GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_DESCRIPTION: &str = "Applies startup/post-chat/reconnect runtime snapshots, verifies chat round-trips and user churn/removals, and completes local chat sends.";
const GUI_SEMANTIC_SCENARIO_PERSISTENCE_RESET_FLOW_SCRIPT: &str = "# Persistence, clear-GUI-data, and config-migration flow\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\thost\tpersisted.example\nsetting\troom\tPersistenceRoom\nsetting\tplayer-path\tC:/Windows/System32/notepad.exe\n";
const GUI_SEMANTIC_SCENARIO_PERSISTENCE_RESET_FLOW_DESCRIPTION: &str = "Seeds legacy GUI-side state next to syncplay.ini, verifies non-INI restore on startup, runs the clear-GUI-data flow through the runtime owner, and proves GUI-owned public-server state wins predictably over conflicting syncplay.ini rows during migration.";
const GUI_SEMANTIC_SCENARIO_DETACHED_RUNTIME_OWNERSHIP_FLOW_SCRIPT: &str = "# Detached runtime ownership flow\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\tusername\tsemantic-user\nsetting\troom\tsemantic-room\nsetting\tpublic-server\tPrimary\t127.0.0.1:8999\nsetting\tmedia-search-directory\tC:/Media\n";
const GUI_SEMANTIC_SCENARIO_DETACHED_RUNTIME_OWNERSHIP_FLOW_DESCRIPTION: &str = "Bootstraps detached public-server connect from GUI state against a local mock server, refreshes browser rows without a preexisting session, and searches missing media from detached GUI playlist state.";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONNECT_FLOW_SCRIPT: &str = "# Live Python reference-peer connect, readiness, chat, playlist, and reconnect flow against the legacy Syncplay server\n# Peer: interop-py-peer\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\tusername\tinterop-gui-user\nsetting\troom\tinterop-room\nsetting\tshared-playlist-enabled\ttrue\n";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONNECT_FLOW_DESCRIPTION: &str = "Connects the GUI runtime to a live legacy Syncplay server that already has a Python reference peer attached, switches the GUI between rooms and back, verifies shared-room projection plus bidirectional readiness, chat, and playlist propagation, then forces a transient peer disconnect/reconnect and re-validates post-reconnect chat.";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONTROLLED_ROOM_FLOW_SCRIPT: &str = "# Live Python reference-peer controlled-room flow against the legacy Syncplay server\n# Peer: interop-py-peer\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\tusername\tinterop-gui-user\nsetting\troom\t+interop-room:447CE7E3548D:AB-123-456\nsetting\tshared-playlist-enabled\ttrue\n";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONTROLLED_ROOM_FLOW_DESCRIPTION: &str = "Connects the GUI runtime to a live legacy Syncplay server in a controlled room, auto-authenticates the GUI as controller from the stored room password, and verifies controller-state projection plus controller-only playlist enablement against the Python reference peer.";

fn normalize_script_line_endings(script: &str) -> String {
    script.replace("\r\n", "\n").replace('\r', "\n")
}

fn normalized_builtin_script(
    raw_script: &'static str,
    cache: &'static OnceLock<String>,
) -> &'static str {
    cache
        .get_or_init(|| normalize_script_line_endings(raw_script))
        .as_str()
}

fn gui_semantic_scenario_from_builtin_script(
    name: &'static str,
    script_source_label: &str,
    script: &str,
) -> GuiSemanticScenario {
    let (_, initial_settings, step_script) =
        parse_external_semantic_script(script).unwrap_or_else(|error| {
            panic!("failed to parse built-in semantic scenario {script_source_label}: {error}")
        });
    GuiSemanticScenario::from_script(name, initial_settings, &step_script).unwrap_or_else(|error| {
        panic!("failed to build built-in semantic scenario {script_source_label}: {error}")
    })
}

#[allow(dead_code)]
pub(crate) fn gui_semantic_scenario_script(name: &str) -> Option<&'static str> {
    match name {
        "configuration-surface-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_SCRIPT_NORMALIZED,
        )),
        "core-shell-smoke-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_SCRIPT_NORMALIZED,
        )),
        "runtime-chat-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT_NORMALIZED,
        )),
        "runtime-transport-churn-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT_NORMALIZED,
        )),
        "persistence-reset-flow" => Some(GUI_SEMANTIC_SCENARIO_PERSISTENCE_RESET_FLOW_SCRIPT),
        "detached-runtime-ownership-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_DETACHED_RUNTIME_OWNERSHIP_FLOW_SCRIPT)
        }
        "live-python-peer-connect-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONNECT_FLOW_SCRIPT)
        }
        "live-python-peer-controlled-room-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONTROLLED_ROOM_FLOW_SCRIPT)
        }
        _ => None,
    }
}

fn gui_semantic_scenario_description(name: &str) -> Option<&'static str> {
    match name {
        "configuration-surface-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_DESCRIPTION)
        }
        "core-shell-smoke-flow" => Some(GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_DESCRIPTION),
        "runtime-chat-flow" => Some(GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_DESCRIPTION),
        "runtime-transport-churn-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_DESCRIPTION)
        }
        "persistence-reset-flow" => Some(GUI_SEMANTIC_SCENARIO_PERSISTENCE_RESET_FLOW_DESCRIPTION),
        "detached-runtime-ownership-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_DETACHED_RUNTIME_OWNERSHIP_FLOW_DESCRIPTION)
        }
        "live-python-peer-connect-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONNECT_FLOW_DESCRIPTION)
        }
        "live-python-peer-controlled-room-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONTROLLED_ROOM_FLOW_DESCRIPTION)
        }
        _ => None,
    }
}

pub(crate) fn gui_semantic_scenario_descriptors() -> Vec<GuiSemanticScenarioDescriptor> {
    gui_semantic_scenario_names()
        .iter()
        .map(|name| GuiSemanticScenarioDescriptor {
            name,
            description: gui_semantic_scenario_description(name)
                .expect("built-in semantic scenario description should exist"),
            script: gui_semantic_scenario_script(name)
                .expect("built-in semantic scenario script should exist"),
        })
        .collect()
}

fn render_gui_semantic_scenario_catalog(format: GuiSemanticOutputFormat) -> String {
    let descriptors = gui_semantic_scenario_descriptors();
    match format {
        GuiSemanticOutputFormat::Text => descriptors
            .into_iter()
            .map(|descriptor| {
                format!(
                    "name={}\ndescription={}\nscript=\n{}\n",
                    descriptor.name, descriptor.description, descriptor.script
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        GuiSemanticOutputFormat::Json => {
            let entries = descriptors
                .into_iter()
                .map(|descriptor| {
                    format!(
                        "{{\"name\":{},\"description\":{},\"script\":{}}}",
                        render_json_string(descriptor.name),
                        render_json_string(descriptor.description),
                        render_json_string(descriptor.script),
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{\"result\":\"ok\",\"scenarios\":[{entries}]}}\n")
        }
    }
}

pub fn run_syncplay_gui_semantic_scenario_catalog(format: GuiSemanticOutputFormat) -> String {
    render_gui_semantic_scenario_catalog(format)
}

pub(super) fn gui_semantic_scenario_configuration_surface_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "configuration-surface-flow",
        "configuration-surface-flow",
        gui_semantic_scenario_script("configuration-surface-flow")
            .expect("configuration semantic scenario script should exist"),
    )
}

pub(super) fn gui_semantic_scenario_core_shell_smoke_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "core-shell-smoke-flow",
        "core-shell-smoke-flow",
        gui_semantic_scenario_script("core-shell-smoke-flow")
            .expect("core shell smoke semantic scenario script should exist"),
    )
}

pub(crate) fn gui_semantic_scenario_names() -> &'static [&'static str] {
    &[
        "configuration-surface-flow",
        "core-shell-smoke-flow",
        "runtime-chat-flow",
        "runtime-transport-churn-flow",
        "persistence-reset-flow",
        "detached-runtime-ownership-flow",
        "live-python-peer-connect-flow",
        "live-python-peer-controlled-room-flow",
    ]
}

pub(super) fn gui_semantic_scenario_named(name: &str) -> Option<GuiSemanticScenario> {
    match name {
        "configuration-surface-flow" => Some(gui_semantic_scenario_configuration_surface_flow()),
        "core-shell-smoke-flow" => Some(gui_semantic_scenario_core_shell_smoke_flow()),
        "runtime-chat-flow" => Some(gui_semantic_scenario_runtime_chat_flow()),
        "runtime-transport-churn-flow" => {
            Some(gui_semantic_scenario_runtime_transport_churn_flow())
        }
        _ => None,
    }
}

pub(super) fn gui_semantic_scenario_name_from_lookup<F>(lookup: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup("SYNCPLAY_GUI_SEMANTIC_SCENARIO")
}

pub(super) fn gui_semantic_scenario_script_path_from_lookup<F>(lookup: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup("SYNCPLAY_GUI_SEMANTIC_SCENARIO_PATH")
}

fn parse_gui_semantic_output_format(
    value: Option<&str>,
) -> Result<GuiSemanticOutputFormat, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("text") => Ok(GuiSemanticOutputFormat::Text),
        Some("json") => Ok(GuiSemanticOutputFormat::Json),
        Some(value) => Err(format!(
            "unknown semantic output format {value:?}. Expected 'text' or 'json'"
        )),
    }
}

pub(super) fn gui_semantic_output_format_from_lookup<F>(
    lookup: F,
) -> Result<GuiSemanticOutputFormat, String>
where
    F: Fn(&str) -> Option<String>,
{
    parse_gui_semantic_output_format(lookup("SYNCPLAY_GUI_SEMANTIC_OUTPUT").as_deref())
}

pub(super) fn run_gui_semantic_scenario_named(
    name: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    if name == "live-python-peer-connect-flow" {
        return run_gui_semantic_live_python_peer_connect_flow();
    }
    if name == "live-python-peer-controlled-room-flow" {
        return run_gui_semantic_live_python_peer_controlled_room_flow();
    }
    if name == "persistence-reset-flow" {
        return run_gui_semantic_persistence_reset_flow();
    }
    if name == "detached-runtime-ownership-flow" {
        return run_gui_semantic_detached_runtime_ownership_flow();
    }
    let scenario = gui_semantic_scenario_named(name).ok_or_else(|| {
        format!(
            "unknown semantic scenario {name:?}. Available: {}",
            gui_semantic_scenario_names().join(", ")
        )
    })?;
    run_gui_semantic_scenario(scenario)
}

fn run_gui_semantic_external_script(
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

fn read_semantic_script_from_path(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read semantic scenario script {path:?}: {error}"))
}

pub fn run_syncplay_gui_semantic_report_from_named_with_append_script_path(
    name: &str,
    append_script_path: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    if matches!(
        name,
        "detached-runtime-ownership-flow"
            | "live-python-peer-connect-flow"
            | "live-python-peer-controlled-room-flow"
    ) {
        return Err(format!(
            "--append-script does not support custom semantic scenario {name:?}"
        ));
    }
    let base_script = gui_semantic_scenario_script(name).ok_or_else(|| {
        format!(
            "unknown semantic scenario {name:?}. Available: {}",
            gui_semantic_scenario_names().join(", ")
        )
    })?;
    let append_script = read_semantic_script_from_path(append_script_path)?;
    let mut combined_script = String::with_capacity(base_script.len() + append_script.len() + 1);
    combined_script.push_str(base_script);
    if !base_script.ends_with('\n') {
        combined_script.push('\n');
    }
    combined_script.push_str(&append_script);
    let mut report = run_gui_semantic_external_script(
        &format!("{name:?}+{append_script_path:?}"),
        &combined_script,
    )?;
    if report.scenario == "external-script" {
        report.scenario = name.to_owned();
    }
    Ok(report)
}

pub fn run_syncplay_gui_semantic_report(
    source: GuiSemanticScenarioSource,
) -> Result<GuiSemanticScenarioReport, String> {
    match source {
        GuiSemanticScenarioSource::Named(name) => run_gui_semantic_scenario_named(&name),
        GuiSemanticScenarioSource::ScriptPath(path) => {
            let script = read_semantic_script_from_path(&path)?;
            let mut report = run_gui_semantic_external_script(&format!("{path:?}"), &script)?;
            if report.scenario == "external-script" {
                report.scenario = path;
            }
            Ok(report)
        }
        GuiSemanticScenarioSource::InlineScript(script) => {
            run_gui_semantic_external_script("inline-script", &script)
        }
    }
}

pub fn run_syncplay_gui_semantic_output(
    source: GuiSemanticScenarioSource,
    format: GuiSemanticOutputFormat,
) -> Result<String, String> {
    run_syncplay_gui_semantic_report(source).map(|report| report.render(format))
}

#[allow(dead_code)]
pub fn run_syncplay_gui_semantic_report_from_script(
    script: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::InlineScript(script.to_owned()))
}

#[allow(dead_code)]
pub fn run_syncplay_gui_semantic_report_from_script_path(
    path: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::ScriptPath(path.to_owned()))
}

#[allow(dead_code)]
pub(crate) fn run_syncplay_gui_semantic_cli_from_args<I, S>(
    args: I,
) -> Result<Option<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut selection: Option<GuiSemanticScenarioSource> = None;
    let mut format = GuiSemanticOutputFormat::Text;
    let mut list = false;
    let mut printed_script: Option<String> = None;
    let mut describe_scenarios = false;
    let mut append_script_path: Option<String> = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_ref() {
            "--scenario" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--scenario requires a scenario name".to_owned())?;
                if selection.is_some() {
                    return Err(
                        "semantic smoke accepts only one of --scenario, --script, or --inline-script"
                            .to_owned(),
                    );
                }
                selection = Some(GuiSemanticScenarioSource::Named(value.as_ref().to_owned()));
            }
            "--script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--script requires a script path".to_owned())?;
                if selection.is_some() {
                    return Err(
                        "semantic smoke accepts only one of --scenario, --script, or --inline-script"
                            .to_owned(),
                    );
                }
                selection = Some(GuiSemanticScenarioSource::ScriptPath(
                    value.as_ref().to_owned(),
                ));
            }
            "--inline-script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--inline-script requires script text".to_owned())?;
                if selection.is_some() {
                    return Err(
                        "semantic smoke accepts only one of --scenario, --script, or --inline-script"
                            .to_owned(),
                    );
                }
                selection = Some(GuiSemanticScenarioSource::InlineScript(
                    value.as_ref().to_owned(),
                ));
            }
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--format requires 'text' or 'json'".to_owned())?;
                format = parse_gui_semantic_output_format(Some(value.as_ref()))?;
            }
            "--list" => {
                list = true;
            }
            "--print-script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--print-script requires a scenario name".to_owned())?;
                if printed_script.is_some() {
                    return Err("--print-script can only be provided once".to_owned());
                }
                printed_script = Some(value.as_ref().to_owned());
            }
            "--describe-scenarios" => {
                describe_scenarios = true;
            }
            "--append-script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--append-script requires a script path".to_owned())?;
                if append_script_path.is_some() {
                    return Err("--append-script can only be provided once".to_owned());
                }
                append_script_path = Some(value.as_ref().to_owned());
            }
            other => {
                return Err(format!("unknown semantic smoke argument {other:?}"));
            }
        }
    }

    if list {
        if selection.is_some()
            || printed_script.is_some()
            || describe_scenarios
            || append_script_path.is_some()
        {
            return Err(
                "--list cannot be combined with --scenario, --script, --inline-script, --print-script, --describe-scenarios, or --append-script"
                    .to_owned(),
            );
        }
        return Ok(Some(format!(
            "{}\n",
            gui_semantic_scenario_names().join("\n")
        )));
    }

    if let Some(name) = printed_script {
        if selection.is_some() || describe_scenarios || append_script_path.is_some() {
            return Err(
                "--print-script cannot be combined with --scenario, --script, --inline-script, --describe-scenarios, or --append-script"
                    .to_owned(),
            );
        }
        let Some(script) = gui_semantic_scenario_script(&name) else {
            return Err(format!(
                "unknown semantic scenario {name:?}. Available: {}",
                gui_semantic_scenario_names().join(", ")
            ));
        };
        return Ok(Some(script.to_owned()));
    }

    if describe_scenarios {
        if selection.is_some() || append_script_path.is_some() {
            return Err(
                "--describe-scenarios cannot be combined with --scenario, --script, --inline-script, or --append-script"
                    .to_owned(),
            );
        }
        return Ok(Some(run_syncplay_gui_semantic_scenario_catalog(format)));
    }

    if let Some(append_script_path) = append_script_path {
        return match selection {
            Some(GuiSemanticScenarioSource::Named(name)) => {
                run_syncplay_gui_semantic_report_from_named_with_append_script_path(
                    &name,
                    &append_script_path,
                )
                .map(|report| Some(report.render(format)))
            }
            Some(_) => Err("--append-script currently supports only --scenario NAME".to_owned()),
            None => Err("--append-script requires --scenario NAME".to_owned()),
        };
    }

    selection
        .map(|selection| run_syncplay_gui_semantic_output(selection, format))
        .transpose()
}

pub(super) fn run_gui_semantic_scenario_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiSemanticScenarioReport>, String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(path) = gui_semantic_scenario_script_path_from_lookup(&lookup) {
        return run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::ScriptPath(path))
            .map(Some);
    }
    let Some(name) = gui_semantic_scenario_name_from_lookup(lookup) else {
        return Ok(None);
    };
    run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::Named(name)).map(Some)
}

pub(super) fn run_gui_semantic_scenario_output_from_lookup<F>(
    lookup: F,
) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Option<String>,
{
    let format = gui_semantic_output_format_from_lookup(&lookup)?;
    run_gui_semantic_scenario_from_lookup(lookup)
        .map(|report| report.map(|report| report.render(format)))
}

#[allow(dead_code)]
pub(crate) fn run_syncplay_gui_semantic_report_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiSemanticScenarioReport>, String>
where
    F: Fn(&str) -> Option<String>,
{
    run_gui_semantic_scenario_from_lookup(lookup)
}

pub(crate) fn run_syncplay_gui_semantic_cli_from_lookup<F>(
    lookup: F,
) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Option<String>,
{
    run_gui_semantic_scenario_output_from_lookup(lookup)
}

fn semantic_env_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[allow(dead_code)]
pub(crate) fn run_syncplay_gui_semantic_report_from_env()
-> Result<Option<GuiSemanticScenarioReport>, String> {
    run_syncplay_gui_semantic_report_from_lookup(semantic_env_trimmed)
}

pub(crate) fn run_syncplay_gui_semantic_cli_from_env() -> Result<Option<String>, String> {
    run_syncplay_gui_semantic_cli_from_lookup(semantic_env_trimmed)
}

pub(super) fn gui_semantic_scenario_runtime_chat_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "runtime-chat-flow",
        "runtime-chat-flow",
        gui_semantic_scenario_script("runtime-chat-flow")
            .expect("runtime chat semantic scenario script should exist"),
    )
}

pub(super) fn gui_semantic_scenario_runtime_transport_churn_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "runtime-transport-churn-flow",
        "runtime-transport-churn-flow",
        gui_semantic_scenario_script("runtime-transport-churn-flow")
            .expect("runtime transport churn semantic scenario script should exist"),
    )
}

fn run_gui_semantic_persistence_reset_flow() -> Result<GuiSemanticScenarioReport, String> {
    #[derive(Default)]
    struct RecordingHost;

    impl super::GuiAppHost for RecordingHost {
        type Output = super::SyncplayGuiShellAppState;

        fn render(&mut self, state: super::SyncplayGuiShellAppState) -> Self::Output {
            state
        }
    }

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-semantic-persistence-reset-{}-{unique_suffix}",
        std::process::id()
    ));
    let result = (|| -> Result<GuiSemanticScenarioReport, String> {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|error| {
            format!(
                "failed to create semantic persistence-reset temp root {}: {error}",
                root.display()
            )
        })?;
        let config_path = root.join("syncplay.ini");
        let settings = super::StoredClientSettingsMvp {
            host: Some("persisted.example".to_owned()),
            room: Some("PersistenceRoom".to_owned()),
            player_path: Some("C:/Windows/System32/notepad.exe".to_owned()),
            media_search_directories: Some(vec!["C:/Media".to_owned()]),
            ..super::StoredClientSettingsMvp::default()
        };
        super::upsert_syncplay_ini_stored_client_settings_mvp_at_path(&config_path, &settings)
            .map_err(|error| {
                format!(
                    "failed to seed semantic persistence-reset config {}: {error}",
                    config_path.display()
                )
            })?;

        let persisted_ui_state = super::GuiPersistedUiState {
            active_view: Some(super::GuiShellView::PublicServers),
            selected_public_server_address: Some("custom.example:9001".to_owned()),
            selected_media_search_directory: Some("C:/Media".to_owned()),
            last_media_dialog_directory: Some("D:/Dialogs".to_owned()),
            last_checked_for_updates: None,
            public_servers: vec![("Custom".to_owned(), "custom.example:9001".to_owned())],
        };
        super::persist_gui_ui_state_at_root(&root, &persisted_ui_state).map_err(|error| {
            format!(
                "failed to seed semantic persistence-reset GUI state at {}: {error}",
                root.display()
            )
        })?;
        let loaded_ui_state = super::load_gui_ui_state_from_root(&root)?
            .ok_or_else(|| "seeded semantic GUI state was not reloadable".to_owned())?;

        let mut host = RecordingHost;
        let restored_state = super::run_gui_host_with_startup_actions_and_gui_state(
            &settings,
            Some(&loaded_ui_state),
            Vec::new(),
            &mut host,
        );
        if restored_state.active_view != super::GuiShellView::PublicServers {
            return Err(format!(
                "expected persisted semantic startup view public-servers, got {}",
                restored_state.active_view.label()
            ));
        }
        if restored_state.selected_public_server_index() != Some(0) {
            return Err(
                "expected persisted semantic startup to restore the custom server selection"
                    .to_owned(),
            );
        }
        if restored_state.selection.selected_media_search_directory != Some(0) {
            return Err(
                "expected persisted semantic startup to restore the media-search selection"
                    .to_owned(),
            );
        }
        if restored_state.last_media_dialog_directory.as_deref() != Some("D:/Dialogs") {
            return Err(
                "expected persisted semantic startup to restore the last media dialog directory"
                    .to_owned(),
            );
        }
        if restored_state.saved_configuration.public_servers.as_ref()
            != Some(&persisted_ui_state.public_servers)
        {
            return Err(
                "expected persisted semantic startup to promote GUI public servers when syncplay.ini had none"
                    .to_owned(),
            );
        }

        let mut host = RecordingHost;
        let migrated_state = super::run_gui_host_with_startup_actions_and_gui_state(
            &super::StoredClientSettingsMvp {
                host: Some("saved.example".to_owned()),
                port: Some(8999),
                public_servers: Some(vec![("Saved".to_owned(), "saved.example:8999".to_owned())]),
                ..settings.clone()
            },
            Some(&loaded_ui_state),
            Vec::new(),
            &mut host,
        );
        let migrated_servers = migrated_state
            .public_servers
            .servers
            .iter()
            .map(|row| (row.label.clone(), row.address.clone()))
            .collect::<Vec<_>>();
        if migrated_servers != vec![("Custom".to_owned(), "custom.example:9001".to_owned())] {
            return Err(format!(
                "expected GUI-owned public servers to win over syncplay.ini migration rows, got {:?}",
                migrated_servers
            ));
        }

        let mut owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(Some(config_path));
        let handle = super::GuiQueuedRuntimeBridgeHandle::default();
        let mut clear_state = restored_state;
        if !clear_state.apply(super::GuiShellAction::BeginClearGuiData) {
            return Err("failed to begin semantic clear-GUI-data flow".to_owned());
        }
        handle.push_request(super::GuiRuntimeRequest::CompletePendingOperation(
            super::GuiPendingCompletionRequest::ClearGuiData,
        ));
        super::GuiQueuedRuntimeOwner::pump(&mut owner, &handle, &clear_state);
        let actions = handle.drain_actions();
        if !actions
            .iter()
            .any(|action| matches!(action, super::GuiShellAction::CompleteClearGuiData))
        {
            return Err(
                "semantic clear-GUI-data flow did not produce a completion action".to_owned(),
            );
        }
        for action in actions {
            if !clear_state.apply(action) {
                return Err("semantic clear-GUI-data completion action was rejected".to_owned());
            }
        }

        if root.join("syncplay.ini").exists() {
            return Err("semantic clear-GUI-data flow did not remove syncplay.ini".to_owned());
        }
        if super::load_gui_ui_state_from_root(&root)?.is_some() {
            return Err(
                "semantic clear-GUI-data flow did not remove the persisted GUI state stores"
                    .to_owned(),
            );
        }
        if clear_state.configuration.launch_mode != super::GuiLaunchMode::FirstRun {
            return Err(
                "semantic clear-GUI-data flow did not restore first-run launch mode".to_owned(),
            );
        }
        if clear_state.active_view != super::GuiShellView::Configuration {
            return Err(
                "semantic clear-GUI-data flow did not restore the configuration view".to_owned(),
            );
        }
        if clear_state.saved_configuration != super::StoredClientSettingsMvp::default() {
            return Err(
                "semantic clear-GUI-data flow did not restore the default saved configuration"
                    .to_owned(),
            );
        }

        Ok(GuiSemanticScenarioReport {
            scenario: "persistence-reset-flow".to_owned(),
            view: clear_state.active_view.label().to_owned(),
            modal: clear_state
                .open_modal
                .map(|modal| modal.label().to_owned())
                .unwrap_or_else(|| "none".to_owned()),
            pending: clear_state
                .pending_operation
                .as_ref()
                .map(|pending| pending.kind.label().to_owned())
                .unwrap_or_else(|| "none".to_owned()),
            widgets: clear_state.shell_widget_tree().node_count(),
        })
    })();
    let _ = std::fs::remove_dir_all(&root);
    result
}

fn run_gui_semantic_detached_runtime_ownership_flow() -> Result<GuiSemanticScenarioReport, String> {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("failed to bind detached semantic TCP listener: {error}"))?;
    let address = listener.local_addr().map_err(|error| {
        format!("failed to read detached semantic TCP listener address: {error}")
    })?;
    let (hello_tx, hello_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server_thread = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("detached semantic TCP listener accept failed: {error}"))?;
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .map_err(|error| format!("detached semantic TCP stream clone failed: {error}"))?,
        );
        let mut hello_line = String::new();
        reader
            .read_line(&mut hello_line)
            .map_err(|error| format!("detached semantic TCP hello read failed: {error}"))?;
        hello_tx
            .send(hello_line)
            .map_err(|error| format!("detached semantic TCP hello report failed: {error}"))?;
        for line in [
            r#"{"Hello":{"username":"semantic-user","room":{"name":"semantic-room"},"version":"1.7.5","features":{"chat":true}}}"#,
            r#"{"Set":{"playlistChange":{"files":["missing-source.mkv","missing-target.mkv"],"user":"semantic-user"}}}"#,
            r#"{"Set":{"playlistIndex":{"index":1,"user":"semantic-user"}}}"#,
        ] {
            stream
                .write_all(line.as_bytes())
                .map_err(|error| format!("detached semantic TCP write failed: {error}"))?;
            stream.write_all(b"\r\n").map_err(|error| {
                format!("detached semantic TCP line termination failed: {error}")
            })?;
        }
        stream
            .flush()
            .map_err(|error| format!("detached semantic TCP flush failed: {error}"))?;
        release_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| format!("detached semantic TCP release wait failed: {error}"))?;
        Ok(())
    });

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "syncplay-gui-semantic-detached-runtime-{}-{unique_suffix}",
        std::process::id()
    ));
    let result = (|| -> Result<GuiSemanticScenarioReport, String> {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|error| {
            format!(
                "failed to create detached semantic temp root {}: {error}",
                root.display()
            )
        })?;
        let found_path = root.join("missing-target.mkv");
        std::fs::write(&found_path, b"semantic-detached-runtime-target").map_err(|error| {
            format!(
                "failed to create detached semantic target {}: {error}",
                found_path.display()
            )
        })?;
        let found_path_text = found_path.display().to_string();

        let mut connect_owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let connect_handle = super::GuiQueuedRuntimeBridgeHandle::default();
        let mut connect_state = super::SyncplayGuiShellAppState::from_stored_settings(
            &super::StoredClientSettingsMvp {
                username: Some("semantic-user".to_owned()),
                room: Some("semantic-room".to_owned()),
                public_servers: Some(vec![("Primary".to_owned(), address.to_string())]),
                ..super::StoredClientSettingsMvp::default()
            },
        );

        if !connect_state.apply(super::GuiShellAction::SelectPublicServer(0))
            || !connect_state.apply(super::GuiShellAction::BeginSelectedPublicServerConnect)
        {
            return Err(
                "detached semantic connect flow could not stage the selected public-server connect"
                    .to_owned(),
            );
        }
        connect_handle.push_request(super::GuiRuntimeRequest::CompletePendingOperation(
            super::GuiPendingCompletionRequest::ConnectPublicServer,
        ));
        super::GuiQueuedRuntimeOwner::pump(&mut connect_owner, &connect_handle, &connect_state);
        let connect_actions = connect_handle.drain_actions();
        if !connect_actions.iter().any(|action| {
            matches!(
                action,
                super::GuiShellAction::CompleteSelectedPublicServerConnect
            )
        }) {
            return Err(
                "detached semantic connect flow did not complete through the runtime owner"
                    .to_owned(),
            );
        }
        for action in connect_actions {
            if !connect_state.apply(action) {
                return Err(
                    "detached semantic connect flow rejected a runtime completion action"
                        .to_owned(),
                );
            }
        }
        let hello_line = hello_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| {
                format!("detached semantic connect flow did not observe a GUI hello: {error}")
            })?;
        if !hello_line.contains("\"semantic-user\"") || !hello_line.contains("\"semantic-room\"") {
            return Err(format!(
                "detached semantic connect flow emitted an unexpected hello payload: {hello_line:?}"
            ));
        }

        super::GuiQueuedRuntimeOwner::pump(&mut connect_owner, &connect_handle, &connect_state);
        let hello_actions = connect_handle.drain_actions();
        if !hello_actions.iter().any(|action| {
            matches!(
                action,
                super::GuiShellAction::ApplyMainWindowRuntimeSnapshot(snapshot)
                    if snapshot.room_name == "semantic-room"
                        && snapshot
                            .playlist
                            .iter()
                            .any(|item| item == "missing-target.mkv")
            )
        }) {
            return Err(
                "detached semantic connect flow did not project the mock server playlist"
                    .to_owned(),
            );
        }
        for action in hello_actions {
            if !connect_state.apply(action) {
                return Err(
                    "detached semantic connect flow rejected a projected session action".to_owned(),
                );
            }
        }

        let mut refresh_owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let refresh_handle = super::GuiQueuedRuntimeBridgeHandle::default();
        let mut refresh_state = super::SyncplayGuiShellAppState::from_stored_settings(
            &super::StoredClientSettingsMvp {
                public_servers: Some(vec![
                    (" Primary ".to_owned(), " syncplay.pl:8999 ".to_owned()),
                    ("Duplicate".to_owned(), "SYNCPLAY.PL:8999".to_owned()),
                    ("Backup".to_owned(), "backup.example:9000".to_owned()),
                ]),
                ..super::StoredClientSettingsMvp::default()
            },
        );
        if !refresh_state.apply(super::GuiShellAction::BeginPublicServerRefresh) {
            return Err("detached semantic refresh flow could not begin refresh".to_owned());
        }
        refresh_handle.push_request(super::GuiRuntimeRequest::CompletePendingOperation(
            super::GuiPendingCompletionRequest::RefreshPublicServers(vec![
                (
                    " Detached Primary ".to_owned(),
                    " detached.example:8999 ".to_owned(),
                ),
                ("Duplicate".to_owned(), "DETACHED.EXAMPLE:8999".to_owned()),
                (
                    "Detached Backup".to_owned(),
                    "backup.example:9000".to_owned(),
                ),
            ]),
        ));
        super::GuiQueuedRuntimeOwner::pump(&mut refresh_owner, &refresh_handle, &refresh_state);
        let refresh_actions = refresh_handle.drain_actions();
        if !refresh_actions.iter().any(|action| {
            matches!(
                action,
                super::GuiShellAction::CompletePublicServerRefresh(servers)
                    if servers
                        == &vec![
                            ("Detached Primary".to_owned(), "detached.example:8999".to_owned()),
                            ("Detached Backup".to_owned(), "backup.example:9000".to_owned()),
                        ]
            )
        }) {
            return Err(
                "detached semantic refresh flow did not normalize the disconnected public-server rows"
                    .to_owned(),
            );
        }
        for action in refresh_actions {
            if !refresh_state.apply(action) {
                return Err(
                    "detached semantic refresh flow rejected a refresh completion action"
                        .to_owned(),
                );
            }
        }

        let mut search_owner = super::GuiPersistedConfigRuntimeOwner::with_config_path(None);
        let search_handle = super::GuiQueuedRuntimeBridgeHandle::default();
        let mut search_state = super::SyncplayGuiShellAppState::from_stored_settings(
            &super::StoredClientSettingsMvp {
                media_search_directories: Some(vec![root.display().to_string()]),
                shared_playlist_enabled: Some(true),
                ..super::StoredClientSettingsMvp::default()
            },
        );
        if !search_state.apply(super::GuiShellAction::SwitchView(
            super::GuiShellView::MediaSearch,
        )) || !search_state.apply(super::GuiShellAction::AnnounceSharedPlaylistLoaded(vec![
            "missing-target.mkv".to_owned(),
        ])) || !search_state.apply(super::GuiShellAction::BeginMissingMediaSearch)
        {
            return Err("detached semantic search flow could not stage the search".to_owned());
        }
        search_handle.push_request(super::GuiRuntimeRequest::CompletePendingOperation(
            super::GuiPendingCompletionRequest::SearchMissingMedia,
        ));
        super::GuiQueuedRuntimeOwner::pump(&mut search_owner, &search_handle, &search_state);
        let search_actions = search_handle.drain_actions();
        if search_actions
            != vec![super::GuiShellAction::CompleteMissingMediaSearch(Some(
                found_path_text.clone(),
            ))]
        {
            return Err(format!(
                "detached semantic search flow returned unexpected actions: {search_actions:?}"
            ));
        }
        for action in search_actions {
            if !search_state.apply(action) {
                return Err(
                    "detached semantic search flow rejected a search completion action".to_owned(),
                );
            }
        }
        let expected_search_message = format!("Missing media found: {found_path_text}.");
        if search_state
            .notifications
            .last()
            .map(|notification| notification.message.as_str())
            != Some(expected_search_message.as_str())
        {
            return Err(
                "detached semantic search flow did not surface the located media notification"
                    .to_owned(),
            );
        }

        Ok(GuiSemanticScenarioReport {
            scenario: "detached-runtime-ownership-flow".to_owned(),
            view: search_state.active_view.label().to_owned(),
            modal: search_state
                .open_modal
                .map(|modal| modal.label().to_owned())
                .unwrap_or_else(|| "none".to_owned()),
            pending: search_state
                .pending_operation
                .as_ref()
                .map(|pending| pending.kind.label().to_owned())
                .unwrap_or_else(|| "none".to_owned()),
            widgets: search_state.shell_widget_tree().node_count(),
        })
    })();

    let _ = release_tx.send(());
    let server_result = server_thread
        .join()
        .map_err(|_| "detached semantic TCP server thread panicked".to_owned())?;
    let _ = std::fs::remove_dir_all(&root);
    match (result, server_result) {
        (Ok(report), Ok(())) => Ok(report),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(server_error)) => Err(format!("{error}; {server_error}")),
    }
}

fn run_gui_semantic_live_python_peer_connect_flow() -> Result<GuiSemanticScenarioReport, String> {
    let result = super::live_python_interop::run_live_python_peer_connect_flow()
        .map_err(|error| error.to_string())?;
    Ok(GuiSemanticScenarioReport {
        scenario: "live-python-peer-connect-flow".to_owned(),
        view: "main-window".to_owned(),
        modal: "none".to_owned(),
        pending: "none".to_owned(),
        widgets: result.widget_count,
    })
}

fn run_gui_semantic_live_python_peer_controlled_room_flow()
-> Result<GuiSemanticScenarioReport, String> {
    let result = super::live_python_interop::run_live_python_peer_controlled_room_flow()
        .map_err(|error| error.to_string())?;
    Ok(GuiSemanticScenarioReport {
        scenario: "live-python-peer-controlled-room-flow".to_owned(),
        view: "main-window".to_owned(),
        modal: "none".to_owned(),
        pending: "none".to_owned(),
        widgets: result.widget_count,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn normalize_script_line_endings_converts_crlf_to_lf() {
        let raw = "# header\r\nsetting\tpublic-server\tPrimary\tsyncplay.pl:8999\r\n";
        assert_eq!(
            super::normalize_script_line_endings(raw),
            "# header\nsetting\tpublic-server\tPrimary\tsyncplay.pl:8999\n"
        );
    }
}
