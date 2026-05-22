use std::sync::OnceLock;

use super::super::semantic_driver::GuiSemanticScenario;
use super::external_script::parse_external_semantic_script;
use super::{GuiSemanticOutputFormat, GuiSemanticScenarioDescriptor, render_json_string};

const GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/configuration-surface-flow.txt");
static GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_CONFIGURATION_SURFACE_FLOW_DESCRIPTION: &str = "Edits configuration fields, surfaces validation and command availability, saves, then exercises public-server and media-search pending flows.";
const GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/core-shell-smoke-flow.txt");
static GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_CORE_SHELL_SMOKE_FLOW_DESCRIPTION: &str =
    "Ports the non-transport Windows smoke path into a platform-neutral shell scenario.";
const GUI_SEMANTIC_SCENARIO_LOCALIZED_RUNTIME_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/localized-runtime-flow.txt");
static GUI_SEMANTIC_SCENARIO_LOCALIZED_RUNTIME_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_LOCALIZED_RUNTIME_FLOW_DESCRIPTION: &str = "Selects a non-English GUI language, then verifies localized public-server refresh and update-check runtime text.";
const GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/runtime-chat-flow.txt");
static GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_DESCRIPTION: &str =
    "Applies runtime session state, verifies playlist projection, and completes a local chat send.";
const GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/runtime-transport-churn-flow.txt");
static GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_DESCRIPTION: &str = "Applies startup/post-chat/reconnect runtime snapshots, verifies chat round-trips and user churn/removals, and completes local chat sends.";
const GUI_SEMANTIC_SCENARIO_DRAG_AND_DROP_INGEST_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/drag-and-drop-ingest-flow.txt");
static GUI_SEMANTIC_SCENARIO_DRAG_AND_DROP_INGEST_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_DRAG_AND_DROP_INGEST_FLOW_DESCRIPTION: &str = "Exercises desktop dropped-file routing so window drops ingest shared-playlist media by default while playlist-surface drops keep playlist ingest and playlist-file import behavior.";
const GUI_SEMANTIC_SCENARIO_PLAYLIST_WORKFLOW_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/playlist-workflow-flow.txt");
static GUI_SEMANTIC_SCENARIO_PLAYLIST_WORKFLOW_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_PLAYLIST_WORKFLOW_FLOW_DESCRIPTION: &str = "Exercises the playlist editor, playlist URL editor, undo flow, and detached open-URL editor from a runtime-backed main window.";
const GUI_SEMANTIC_SCENARIO_PLAYER_SETUP_FLOW_SCRIPT: &str =
    include_str!("../../semantic_scenarios/player-setup-flow.txt");
static GUI_SEMANTIC_SCENARIO_PLAYER_SETUP_FLOW_SCRIPT_NORMALIZED: OnceLock<String> =
    OnceLock::new();
const GUI_SEMANTIC_SCENARIO_PLAYER_SETUP_FLOW_DESCRIPTION: &str = "Applies first-run and recovery mpv setup runtime issues, verifies blocking modal behavior, then exercises Retry mpv through the semantic runtime dispatch path.";
const GUI_SEMANTIC_SCENARIO_PERSISTENCE_RESET_FLOW_SCRIPT: &str = "# Persistence, clear-GUI-data, and config-migration flow\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\thost\tpersisted.example\nsetting\troom\tPersistenceRoom\nsetting\tplayer-path\tC:/Windows/System32/notepad.exe\n";
const GUI_SEMANTIC_SCENARIO_PERSISTENCE_RESET_FLOW_DESCRIPTION: &str = "Seeds legacy GUI-side state next to sorotte.ini, verifies non-INI restore on startup, runs the clear-GUI-data flow through the runtime owner, and proves GUI-owned public-server state wins predictably over conflicting sorotte.ini rows during migration.";
const GUI_SEMANTIC_SCENARIO_DETACHED_RUNTIME_OWNERSHIP_FLOW_SCRIPT: &str = "# Detached runtime ownership flow\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\tusername\tsemantic-user\nsetting\troom\tsemantic-room\nsetting\tpublic-server\tPrimary\t127.0.0.1:8999\nsetting\tmedia-search-directory\tC:/Media\n";
const GUI_SEMANTIC_SCENARIO_DETACHED_RUNTIME_OWNERSHIP_FLOW_DESCRIPTION: &str = "Bootstraps detached public-server connect from GUI state against a local mock server, refreshes browser rows without a preexisting session, and searches missing media from detached GUI playlist state.";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONNECT_FLOW_SCRIPT: &str = "# Live Python reference-peer connect, readiness, chat, playlist, and reconnect flow against the legacy Syncplay server\n# Peer: interop-py-peer\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\tusername\tinterop-gui-user\nsetting\troom\tinterop-room\nsetting\tshared-playlist-enabled\ttrue\n";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONNECT_FLOW_DESCRIPTION: &str = "Connects the GUI runtime to a live legacy Syncplay server that already has a Python reference peer attached, switches the GUI between rooms and back, verifies shared-room projection plus bidirectional readiness, chat, and playlist propagation, then forces a transient peer disconnect/reconnect and re-validates post-reconnect chat.";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONTROLLED_ROOM_FLOW_SCRIPT: &str = "# Live Python reference-peer controlled-room flow against the legacy Syncplay server\n# Peer: interop-py-peer\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\nsetting\tusername\tinterop-gui-user\nsetting\troom\t+interop-room:447CE7E3548D:AB-123-456\nsetting\tshared-playlist-enabled\ttrue\n";
const GUI_SEMANTIC_SCENARIO_LIVE_PYTHON_PEER_CONTROLLED_ROOM_FLOW_DESCRIPTION: &str = "Connects the GUI runtime to a live legacy Syncplay server in a controlled room, auto-authenticates the GUI as controller from the stored room password, and verifies controller-state projection plus controller-only playlist enablement against the Python reference peer.";

pub(super) fn normalize_script_line_endings(script: &str) -> String {
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
        "localized-runtime-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_LOCALIZED_RUNTIME_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_LOCALIZED_RUNTIME_FLOW_SCRIPT_NORMALIZED,
        )),
        "runtime-chat-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_SCRIPT_NORMALIZED,
        )),
        "runtime-transport-churn-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_SCRIPT_NORMALIZED,
        )),
        "drag-and-drop-ingest-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_DRAG_AND_DROP_INGEST_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_DRAG_AND_DROP_INGEST_FLOW_SCRIPT_NORMALIZED,
        )),
        "playlist-workflow-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_PLAYLIST_WORKFLOW_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_PLAYLIST_WORKFLOW_FLOW_SCRIPT_NORMALIZED,
        )),
        "player-setup-flow" => Some(normalized_builtin_script(
            GUI_SEMANTIC_SCENARIO_PLAYER_SETUP_FLOW_SCRIPT,
            &GUI_SEMANTIC_SCENARIO_PLAYER_SETUP_FLOW_SCRIPT_NORMALIZED,
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
        "localized-runtime-flow" => Some(GUI_SEMANTIC_SCENARIO_LOCALIZED_RUNTIME_FLOW_DESCRIPTION),
        "runtime-chat-flow" => Some(GUI_SEMANTIC_SCENARIO_RUNTIME_CHAT_FLOW_DESCRIPTION),
        "runtime-transport-churn-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_RUNTIME_TRANSPORT_CHURN_FLOW_DESCRIPTION)
        }
        "drag-and-drop-ingest-flow" => {
            Some(GUI_SEMANTIC_SCENARIO_DRAG_AND_DROP_INGEST_FLOW_DESCRIPTION)
        }
        "playlist-workflow-flow" => Some(GUI_SEMANTIC_SCENARIO_PLAYLIST_WORKFLOW_FLOW_DESCRIPTION),
        "player-setup-flow" => Some(GUI_SEMANTIC_SCENARIO_PLAYER_SETUP_FLOW_DESCRIPTION),
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

pub(super) fn run_sorotte_gui_semantic_scenario_catalog(format: GuiSemanticOutputFormat) -> String {
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

pub(super) fn gui_semantic_scenario_localized_runtime_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "localized-runtime-flow",
        "localized-runtime-flow",
        gui_semantic_scenario_script("localized-runtime-flow")
            .expect("localized runtime semantic scenario script should exist"),
    )
}

pub(super) fn gui_semantic_scenario_drag_and_drop_ingest_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "drag-and-drop-ingest-flow",
        "drag-and-drop-ingest-flow",
        gui_semantic_scenario_script("drag-and-drop-ingest-flow")
            .expect("drag-and-drop semantic scenario script should exist"),
    )
}

pub(super) fn gui_semantic_scenario_playlist_workflow_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "playlist-workflow-flow",
        "playlist-workflow-flow",
        gui_semantic_scenario_script("playlist-workflow-flow")
            .expect("playlist workflow semantic scenario script should exist"),
    )
}

pub(super) fn gui_semantic_scenario_player_setup_flow() -> GuiSemanticScenario {
    gui_semantic_scenario_from_builtin_script(
        "player-setup-flow",
        "player-setup-flow",
        gui_semantic_scenario_script("player-setup-flow")
            .expect("player setup semantic scenario script should exist"),
    )
}

pub(crate) fn gui_semantic_scenario_names() -> &'static [&'static str] {
    &[
        "configuration-surface-flow",
        "core-shell-smoke-flow",
        "localized-runtime-flow",
        "runtime-chat-flow",
        "runtime-transport-churn-flow",
        "drag-and-drop-ingest-flow",
        "playlist-workflow-flow",
        "player-setup-flow",
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
        "localized-runtime-flow" => Some(gui_semantic_scenario_localized_runtime_flow()),
        "runtime-chat-flow" => Some(gui_semantic_scenario_runtime_chat_flow()),
        "runtime-transport-churn-flow" => {
            Some(gui_semantic_scenario_runtime_transport_churn_flow())
        }
        "drag-and-drop-ingest-flow" => Some(gui_semantic_scenario_drag_and_drop_ingest_flow()),
        "playlist-workflow-flow" => Some(gui_semantic_scenario_playlist_workflow_flow()),
        "player-setup-flow" => Some(gui_semantic_scenario_player_setup_flow()),
        _ => None,
    }
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
