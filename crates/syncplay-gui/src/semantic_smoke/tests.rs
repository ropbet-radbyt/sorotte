use crate::app::render_io::GuiDroppedFilesTarget;
use crate::app::semantic_driver::GuiSemanticStep;

use super::{
    GuiSemanticOutputFormat, GuiSemanticScenarioSource, gui_semantic_output_format_from_lookup,
    gui_semantic_scenario_descriptors, gui_semantic_scenario_name_from_lookup,
    gui_semantic_scenario_named, gui_semantic_scenario_names, gui_semantic_scenario_script,
    normalize_script_line_endings, run_gui_semantic_scenario_from_lookup,
    run_gui_semantic_scenario_named, run_syncplay_gui_semantic_cli_from_args,
    run_syncplay_gui_semantic_cli_from_lookup, run_syncplay_gui_semantic_report,
    run_syncplay_gui_semantic_report_from_lookup,
};

#[test]
fn normalize_script_line_endings_converts_crlf_to_lf() {
    let raw = "# header\r\nsetting\tpublic-server\tPrimary\tsyncplay.pl:8999\r\n";
    assert_eq!(
        normalize_script_line_endings(raw),
        "# header\nsetting\tpublic-server\tPrimary\tsyncplay.pl:8999\n"
    );
}

#[test]
fn gui_semantic_scenarios_expose_named_catalog_and_parse_scripts() {
    assert_eq!(
        gui_semantic_scenario_names(),
        &[
            "configuration-surface-flow",
            "core-shell-smoke-flow",
            "localized-runtime-flow",
            "runtime-chat-flow",
            "runtime-transport-churn-flow",
            "drag-and-drop-ingest-flow",
            "playlist-workflow-flow",
            "persistence-reset-flow",
            "detached-runtime-ownership-flow",
            "live-python-peer-connect-flow",
            "live-python-peer-controlled-room-flow",
        ]
    );
    assert!(
        gui_semantic_scenario_script("configuration-surface-flow")
            .expect("built-in configuration scenario should expose a script")
            .contains("setting\tpublic-server\tPrimary\tsyncplay.pl:8999")
    );
    assert!(
        gui_semantic_scenario_script("core-shell-smoke-flow")
            .expect("built-in core shell smoke scenario should expose a script")
            .contains("close-modal")
    );
    assert!(
        gui_semantic_scenario_script("localized-runtime-flow")
            .expect("localized runtime scenario should expose a script")
            .contains("config:System:Language\tfalse\tfr")
    );
    assert!(
        gui_semantic_scenario_script("runtime-chat-flow")
            .expect("built-in runtime scenario should expose a script")
            .contains("push-chat-message\tbob\thello from tcp")
    );
    assert!(
        gui_semantic_scenario_script("runtime-transport-churn-flow")
            .expect("built-in runtime churn scenario should expose a script")
            .contains("apply-main-window-runtime\tsmoke-room\ttrue\ttrue\tfalse")
    );
    assert!(
        gui_semantic_scenario_script("drag-and-drop-ingest-flow")
            .expect("drag-and-drop scenario should expose a script")
            .contains("drop-media-files\tplaylist")
    );
    assert!(
        gui_semantic_scenario_script("playlist-workflow-flow")
            .expect("playlist workflow scenario should expose a script")
            .contains("main-window:playlist:edit")
    );
    assert!(
        gui_semantic_scenario_script("persistence-reset-flow")
            .expect("persistence/reset scenario should expose a script description")
            .contains("PersistenceRoom")
    );
    assert!(
        gui_semantic_scenario_script("detached-runtime-ownership-flow")
            .expect("detached runtime ownership scenario should expose a script description")
            .contains("semantic-user")
    );
    assert!(
        gui_semantic_scenario_script("live-python-peer-connect-flow")
            .expect("live Python interop scenario should expose a script description")
            .contains("interop-py-peer")
    );
    assert!(
        gui_semantic_scenario_script("live-python-peer-controlled-room-flow")
            .expect("live Python controlled-room scenario should expose a script description")
            .contains("+interop-room:447CE7E3548D:AB-123-456")
    );
    assert!(
        gui_semantic_scenario_script("missing-scenario").is_none(),
        "unknown semantic scenario scripts should not resolve"
    );
    let descriptors = gui_semantic_scenario_descriptors();
    assert_eq!(descriptors.len(), 11);
    assert_eq!(descriptors[0].name, "configuration-surface-flow");
    assert!(descriptors[0].description.contains("configuration fields"));
    assert!(
        descriptors[0]
            .script
            .contains("setting\tpublic-server\tPrimary\tsyncplay.pl:8999")
    );
    assert_eq!(descriptors[1].name, "core-shell-smoke-flow");
    assert!(descriptors[1].description.contains("non-transport"));
    assert!(descriptors[1].script.contains("clear-notifications"));
    assert_eq!(descriptors[2].name, "localized-runtime-flow");
    assert!(
        descriptors[2]
            .description
            .contains("non-English GUI language")
    );
    assert!(
        descriptors[2]
            .script
            .contains("shell:modal:update:message\tSyncplay est a jour")
    );
    assert_eq!(descriptors[4].name, "runtime-transport-churn-flow");
    assert!(
        descriptors[4]
            .description
            .contains("startup/post-chat/reconnect")
    );
    assert!(descriptors[4].script.contains("reconnect-post2.mkv"));
    assert_eq!(descriptors[5].name, "drag-and-drop-ingest-flow");
    assert!(
        descriptors[5]
            .description
            .contains("window drops ingest shared-playlist media")
    );
    assert!(descriptors[5].script.contains("drop-media-files\twindow"));
    assert_eq!(descriptors[6].name, "playlist-workflow-flow");
    assert!(descriptors[6].description.contains("playlist editor"));
    assert!(
        descriptors[6]
            .script
            .contains("main-window:playlist:add-url")
    );
    assert_eq!(descriptors[7].name, "persistence-reset-flow");
    assert!(descriptors[7].description.contains("clear-GUI-data"));
    assert!(descriptors[7].script.contains("PersistenceRoom"));
    assert_eq!(descriptors[8].name, "detached-runtime-ownership-flow");
    assert!(
        descriptors[8]
            .description
            .contains("detached public-server connect")
    );
    assert!(descriptors[8].script.contains("semantic-user"));
    assert_eq!(descriptors[9].name, "live-python-peer-connect-flow");
    assert!(descriptors[9].description.contains("Python reference peer"));
    assert!(descriptors[9].script.contains("interop-room"));
    assert_eq!(
        descriptors[10].name,
        "live-python-peer-controlled-room-flow"
    );
    assert!(descriptors[10].description.contains("controlled room"));
    assert!(
        descriptors[10]
            .script
            .contains("+interop-room:447CE7E3548D")
    );
    assert!(
        gui_semantic_scenario_named("missing-scenario").is_none(),
        "unknown semantic scenarios should not resolve"
    );

    let parsed = GuiSemanticStep::parse_script(
        "\
# comment\n\
activate\tconfiguration-root\n\
assert-selected\tconfiguration-root\ttrue\n\
assert-value\tconfig:Connection:Host\t<none>\n\
assert-pending\tnone\n\
complete-pending\n\
complete-pending-runtime\n\
open-media-files\tC:/Media/open-target.mkv\n\
drop-media-files\tplaylist\tC:/Media/episode1.mkv|C:/Media/episode2.mkv\n\
close-modal\n\
clear-notifications\n",
    )
    .expect("semantic step script should parse");
    assert_eq!(
        parsed,
        vec![
            GuiSemanticStep::activate("configuration-root"),
            GuiSemanticStep::assert_widget_selected("configuration-root", true),
            GuiSemanticStep::assert_widget_value("config:Connection:Host", None),
            GuiSemanticStep::assert_pending(None),
            GuiSemanticStep::CompletePending,
            GuiSemanticStep::CompletePendingRuntime,
            GuiSemanticStep::OpenMediaFiles(vec!["C:/Media/open-target.mkv".to_owned(),]),
            GuiSemanticStep::DropMediaFiles {
                target: GuiDroppedFilesTarget::Playlist,
                paths: vec![
                    "C:/Media/episode1.mkv".to_owned(),
                    "C:/Media/episode2.mkv".to_owned(),
                ],
            },
            GuiSemanticStep::CloseModal,
            GuiSemanticStep::ClearNotifications,
        ]
    );

    let parsed_runtime = GuiSemanticStep::parse_script(
        "\
apply-main-window-runtime\troom-a\ttrue\tfalse\tfalse\ttrue\ttrue\tfalse\ttrue\tself,true,true,false|bob,false,false,true\tvideo1.mkv|video2.mkv\tsystem>connected\n\
apply-main-window-playlist-selection\t1\n\
push-chat-message\tbob\thello\n\
assert-value\tmain-window:chat-input\t<empty>\n",
    )
    .expect("runtime semantic step script should parse");
    assert_eq!(parsed_runtime.len(), 4);
    assert!(matches!(
        &parsed_runtime[0],
        GuiSemanticStep::ApplyMainWindowRuntimeSnapshot(snapshot)
            if snapshot.room_name == "room-a"
            && snapshot.shared_playlist_enabled
            && !snapshot.playback_paused
            && snapshot.playlist == vec!["video1.mkv".to_owned(), "video2.mkv".to_owned()]
            && snapshot.users.len() == 2
            && snapshot.chat.len() == 1
    ));
    assert_eq!(
        parsed_runtime[1],
        GuiSemanticStep::ApplyMainWindowPlaylistSelection(Some(1))
    );
    assert_eq!(
        parsed_runtime[2],
        GuiSemanticStep::PushChatMessage {
            sender: "bob".to_owned(),
            message: "hello".to_owned(),
        }
    );
    assert_eq!(
        parsed_runtime[3],
        GuiSemanticStep::assert_widget_value("main-window:chat-input", Some(""))
    );
}

#[test]
fn gui_semantic_scenario_runner_reports_named_results_from_lookup() {
    assert_eq!(
        gui_semantic_scenario_name_from_lookup(|name| {
            (name == "SYNCPLAY_GUI_SEMANTIC_SCENARIO")
                .then(|| "configuration-surface-flow".to_owned())
        }),
        Some("configuration-surface-flow".to_owned())
    );
    let report = run_gui_semantic_scenario_from_lookup(|name| {
        (name == "SYNCPLAY_GUI_SEMANTIC_SCENARIO").then(|| "configuration-surface-flow".to_owned())
    })
    .expect("named semantic scenario should run")
    .expect("lookup should produce a report");
    assert_eq!(report.scenario, "configuration-surface-flow");
    assert_eq!(report.view, "media-search");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
    assert!(
        report
            .render(GuiSemanticOutputFormat::Text)
            .contains("result=ok\n")
    );

    let json_report = run_syncplay_gui_semantic_cli_from_lookup(|name| match name {
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
        "SYNCPLAY_GUI_SEMANTIC_OUTPUT" => Some("json".to_owned()),
        _ => None,
    })
    .expect("json semantic scenario should run")
    .expect("lookup should produce json output");
    assert!(json_report.starts_with("{\"result\":\"ok\","));
    assert!(json_report.contains("\"scenario\":\"configuration-surface-flow\""));
    assert!(
        gui_semantic_output_format_from_lookup(|name| {
            (name == "SYNCPLAY_GUI_SEMANTIC_OUTPUT").then(|| "yaml".to_owned())
        })
        .expect_err("unknown semantic output format should fail")
        .contains("Expected 'text' or 'json'")
    );

    let mut script_path = std::env::temp_dir();
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    script_path.push(format!(
        "syncplay-gui-semantic-scenario-{}-{unique_id}.txt",
        std::process::id()
    ));
    std::fs::write(
        &script_path,
        "\
meta\tname\tfile-seeded-flow\n\
meta\texpect-view\tpublic-servers\n\
meta\texpect-modal\tnone\n\
meta\texpect-pending\tnone\n\
setting\thost\tfile-script.example\n\
setting\tport\t8999\n\
setting\tpublic-server\tMirror\tmirror.example:8999\n\
assert-selected\tconfiguration-root\ttrue\n\
assert-value\tconfig:Connection:Host\tfile-script.example\n\
assert-value\tconfig:Connection:Port\t8999\n\
activate\tpublic-servers-root\n\
assert-selected\tpublic-servers-root\ttrue\n\
assert-label\tpublic-servers:row:0\tMirror\n",
    )
    .expect("semantic script file should be created");
    let script_path_string = script_path.to_string_lossy().into_owned();
    let script_report = run_gui_semantic_scenario_from_lookup(|name| match name {
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO_PATH" => Some(script_path_string.clone()),
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
        _ => None,
    })
    .expect("script semantic scenario should run")
    .expect("lookup should produce a script report");
    assert_eq!(script_report.scenario, "file-seeded-flow");
    assert_eq!(script_report.view, "public-servers");

    std::fs::write(
        &script_path,
        "\
meta\texpect-view\tmain-window\n\
assert-selected\tconfiguration-root\ttrue\n",
    )
    .expect("mismatch semantic script file should be updated");
    assert!(
        run_gui_semantic_scenario_from_lookup(|name| match name {
            "SYNCPLAY_GUI_SEMANTIC_SCENARIO_PATH" => Some(script_path_string.clone()),
            _ => None,
        })
        .expect_err("mismatched script metadata should fail")
        .contains("expected final view")
    );

    std::fs::remove_file(&script_path).expect("semantic script file should be removed");

    assert!(
        run_gui_semantic_scenario_named("missing-scenario")
            .expect_err("unknown scenario should fail")
            .contains(
                "Available: configuration-surface-flow, core-shell-smoke-flow, localized-runtime-flow, runtime-chat-flow, runtime-transport-churn-flow, drag-and-drop-ingest-flow, playlist-workflow-flow, persistence-reset-flow, detached-runtime-ownership-flow, live-python-peer-connect-flow, live-python-peer-controlled-room-flow"
            )
    );
}

#[test]
fn syncplay_gui_semantic_cli_wrapper_renders_lookup_output() {
    let output = run_syncplay_gui_semantic_cli_from_lookup(|name| match name {
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
        "SYNCPLAY_GUI_SEMANTIC_OUTPUT" => Some("json".to_owned()),
        _ => None,
    })
    .expect("semantic cli wrapper should run")
    .expect("semantic cli wrapper should produce output");
    assert!(output.starts_with("{\"result\":\"ok\","));
    assert!(output.contains("\"view\":\"media-search\""));
}

#[test]
fn syncplay_gui_semantic_cli_wrapper_runs_explicit_args() {
    let output = run_syncplay_gui_semantic_cli_from_args([
        "--scenario",
        "runtime-chat-flow",
        "--format",
        "json",
    ])
    .expect("semantic cli args wrapper should run")
    .expect("semantic cli args wrapper should produce output");
    assert!(output.starts_with("{\"result\":\"ok\","));
    assert!(output.contains("\"scenario\":\"runtime-chat-flow\""));

    let listed = run_syncplay_gui_semantic_cli_from_args(["--list"])
        .expect("semantic cli list should run")
        .expect("semantic cli list should produce output");
    assert!(listed.contains("configuration-surface-flow"));
    assert!(listed.contains("core-shell-smoke-flow"));
    assert!(listed.contains("localized-runtime-flow"));
    assert!(listed.contains("runtime-chat-flow"));
    assert!(listed.contains("runtime-transport-churn-flow"));
    assert!(listed.contains("detached-runtime-ownership-flow"));
    assert!(listed.contains("live-python-peer-connect-flow"));
    assert!(listed.contains("live-python-peer-controlled-room-flow"));

    let printed =
        run_syncplay_gui_semantic_cli_from_args(["--print-script", "configuration-surface-flow"])
            .expect("semantic cli print-script should run")
            .expect("semantic cli print-script should produce output");
    assert!(printed.contains("setting\tpublic-server\tPrimary\tsyncplay.pl:8999"));
    assert!(printed.contains("activate\tmedia-search:command:search"));

    let mut append_script_path = std::env::temp_dir();
    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    append_script_path.push(format!(
        "syncplay-gui-semantic-append-{}-{unique_id}.txt",
        std::process::id()
    ));
    std::fs::write(
        &append_script_path,
        "\
# delta script\n\
enter-text\tconfig:Connection:Host\tfalse\toverride.example\n\
assert-value\tconfig:Connection:Host\toverride.example\n",
    )
    .expect("semantic append script file should be created");
    let append_script_path_string = append_script_path.to_string_lossy().into_owned();
    let appended = run_syncplay_gui_semantic_cli_from_args([
        "--scenario",
        "configuration-surface-flow",
        "--append-script",
        &append_script_path_string,
        "--format",
        "json",
    ])
    .expect("semantic cli append-script should run")
    .expect("semantic cli append-script should produce output");
    assert!(appended.starts_with("{\"result\":\"ok\","));
    assert!(appended.contains("\"scenario\":\"configuration-surface-flow\""));
    assert!(appended.contains("\"view\":\"media-search\""));
    std::fs::remove_file(&append_script_path)
        .expect("semantic append script file should be removed");

    let described =
        run_syncplay_gui_semantic_cli_from_args(["--describe-scenarios", "--format", "json"])
            .expect("semantic cli describe-scenarios should run")
            .expect("semantic cli describe-scenarios should produce output");
    assert!(described.starts_with("{\"result\":\"ok\",\"scenarios\":["));
    assert!(described.contains("\"name\":\"configuration-surface-flow\""));
    assert!(described.contains("\"description\":\"Edits configuration fields, surfaces validation and command availability, saves, then exercises public-server and media-search pending flows.\""));
    assert!(described.contains("\"script\":\"# Configuration save and follow-on cross-surface workflow\\nsetting\\tpublic-server\\tPrimary\\tsyncplay.pl:8999"));
    assert!(described.contains("\"name\":\"core-shell-smoke-flow\""));
    assert!(described.contains("\"description\":\"Ports the non-transport Windows smoke path into a platform-neutral shell scenario.\""));
    assert!(described.contains("\"script\":\"# Core shell smoke flow ported from the legacy non-transport Windows smoke path\\nsetting\\tpublic-server\\tAlpha\\talpha.example:8999"));
    assert!(described.contains("\"name\":\"localized-runtime-flow\""));
    assert!(described.contains("\"description\":\"Selects a non-English GUI language, then verifies localized public-server refresh and update-check runtime text.\""));
    assert!(described.contains("\"script\":\"# Localized runtime and service-call flow\\nsetting\\tpublic-server\\tAlpha\\talpha.example:8999"));
    assert!(described.contains("\"name\":\"runtime-transport-churn-flow\""));
    assert!(described.contains("\"description\":\"Applies startup/post-chat/reconnect runtime snapshots, verifies chat round-trips and user churn/removals, and completes local chat sends.\""));
    assert!(described.contains("\"script\":\"# Runtime-backed transport churn/reconnect flow without platform UI dependencies\\nsetting\\tusername\\tsmoke-user"));
    assert!(described.contains("\"name\":\"live-python-peer-connect-flow\""));
    assert!(described.contains("\"description\":\"Connects the GUI runtime to a live legacy Syncplay server that already has a Python reference peer attached, switches the GUI between rooms and back, verifies shared-room projection plus bidirectional readiness, chat, and playlist propagation, then forces a transient peer disconnect/reconnect and re-validates post-reconnect chat.\""));
    assert!(described.contains("\"script\":\"# Live Python reference-peer connect, readiness, chat, playlist, and reconnect flow against the legacy Syncplay server\\n# Peer: interop-py-peer\\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\\nsetting\\tusername\\tinterop-gui-user\\nsetting\\troom\\tinterop-room\\nsetting\\tshared-playlist-enabled\\ttrue"));
    assert!(described.contains("\"name\":\"live-python-peer-controlled-room-flow\""));
    assert!(described.contains("\"description\":\"Connects the GUI runtime to a live legacy Syncplay server in a controlled room, auto-authenticates the GUI as controller from the stored room password, and verifies controller-state projection plus controller-only playlist enablement against the Python reference peer.\""));
    assert!(described.contains("\"script\":\"# Live Python reference-peer controlled-room flow against the legacy Syncplay server\\n# Peer: interop-py-peer\\n# Executed by a code-driven semantic runner; append-script is not supported for this scenario.\\nsetting\\tusername\\tinterop-gui-user\\nsetting\\troom\\t+interop-room:447CE7E3548D:AB-123-456\\nsetting\\tshared-playlist-enabled\\ttrue"));
}

#[test]
fn syncplay_gui_semantic_report_wrapper_returns_structured_lookup_output() {
    let report = run_syncplay_gui_semantic_report_from_lookup(|name| match name {
        "SYNCPLAY_GUI_SEMANTIC_SCENARIO" => Some("configuration-surface-flow".to_owned()),
        _ => None,
    })
    .expect("semantic report wrapper should run")
    .expect("semantic report wrapper should return a report");
    assert_eq!(report.scenario, "configuration-surface-flow");
    assert_eq!(report.view, "media-search");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
}

#[test]
fn syncplay_gui_semantic_report_wrapper_runs_persistence_reset_flow() {
    let report = run_gui_semantic_scenario_named("persistence-reset-flow")
        .expect("persistence/reset semantic scenario should run");
    assert_eq!(report.scenario, "persistence-reset-flow");
    assert_eq!(report.view, "configuration");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
}

#[test]
fn syncplay_gui_semantic_report_wrapper_runs_inline_script() {
    let report = run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::InlineScript(
        "\
meta\tname\tinline-check\n\
meta\texpect-view\tconfiguration\n\
assert-selected\tconfiguration-root\ttrue\n\
assert-pending\tnone\n"
            .to_owned(),
    ))
    .expect("inline semantic script should run");
    assert_eq!(report.scenario, "inline-check");
    assert_eq!(report.view, "configuration");
    assert_eq!(report.modal, "none");
    assert_eq!(report.pending, "none");
    assert!(report.widgets > 0);
}
