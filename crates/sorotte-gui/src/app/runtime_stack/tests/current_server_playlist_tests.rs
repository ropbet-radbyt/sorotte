use super::*;
use crate::app::runtime_stack::test_support::GuiSessionDeliveryTestExt;

fn exchange(
    adapter: &mut GuiClientCoreChatSessionRuntimeAdapter,
    server: &mut sorotte_server::ServerRuntime,
) {
    for _ in 0..64 {
        let lines = adapter.deliver_outbound_protocol_lines().unwrap();
        if lines.is_empty() {
            return;
        }
        for line in lines {
            for response in server.handle_line("alice", &line).unwrap() {
                adapter.apply_message_json(&response).unwrap();
            }
        }
    }
    panic!("playlist exchange failed to settle");
}

fn assert_playlist(adapter: &GuiClientCoreChatSessionRuntimeAdapter, files: &[&str], index: i64) {
    let canonical = adapter.runtime.session().current_room_playlist().unwrap();
    let projected = adapter.projected_current_room_playlist().unwrap();
    assert_eq!(canonical.files, files, "client session contents");
    assert_eq!(canonical.index, Some(index), "client session selection");
    assert_eq!(
        projected.files, canonical.files,
        "GUI must follow session contents"
    );
    assert_eq!(
        projected.index, canonical.index,
        "GUI must follow session selection"
    );
}

#[test]
fn current_server_append_select_edit_playlist_remains_authoritative() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1").unwrap();
    let state = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        username: Some("alice".into()),
        room: Some("room1".into()),
        shared_playlist_enabled: Some(true),
        ..StoredClientSettings::default()
    });
    sync_adapter_to_saved_session_settings(&mut adapter, &state);
    let mut server = sorotte_server::ServerRuntime::new();
    exchange(&mut adapter, &mut server);
    adapter
        .replace_playlist(vec!["episode1.mkv".into()], Some(0))
        .unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode1.mkv"], 0);
    adapter
        .replace_playlist(vec!["episode1.mkv".into(), "episode2.mkv".into()], Some(0))
        .unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode1.mkv", "episode2.mkv"], 0);
    adapter.set_playlist_index(1).unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode1.mkv", "episode2.mkv"], 1);
    adapter.delete_playlist_index(0).unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode2.mkv"], 0);
    adapter.undo_playlist_change().unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode1.mkv", "episode2.mkv"], 1);
    adapter
        .queue_playlist_entry("episode3.mkv".into(), true)
        .unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(
        &adapter,
        &["episode1.mkv", "episode2.mkv", "episode3.mkv"],
        2,
    );
}

#[test]
fn current_server_pending_append_cannot_mask_newer_selection() {
    let mut adapter = GuiClientCoreChatSessionRuntimeAdapter::new("alice", "room1").unwrap();
    let mut server = sorotte_server::ServerRuntime::new();
    exchange(&mut adapter, &mut server);
    adapter
        .replace_playlist(vec!["episode1.mkv".into()], Some(0))
        .unwrap();
    exchange(&mut adapter, &mut server);
    adapter
        .replace_playlist(vec!["episode1.mkv".into(), "episode2.mkv".into()], Some(0))
        .unwrap();
    adapter.set_playlist_index(1).unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode1.mkv", "episode2.mkv"], 1);
    adapter.delete_playlist_index(0).unwrap();
    exchange(&mut adapter, &mut server);
    assert_playlist(&adapter, &["episode2.mkv"], 0);
}
