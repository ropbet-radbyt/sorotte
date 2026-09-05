use super::*;
use sorotte_protocol::{ListUserEntry, encode_message_line};
use std::collections::BTreeMap;

fn hello(
    runtime: &mut ServerRuntime,
    id: &str,
    room: &str,
    features: Value,
) -> Result<Vec<String>, ServerRuntimeError> {
    runtime.handle_line(id, &json!({"Hello":{"username":id,"room":{"name":room},"version":"1.7.5","features":features}}).to_string())
}

fn large_features() -> Value {
    json!({"sorotteLargeProtocolFramesV1":true,"mediaMatch":true,"uiMode":"GUI"})
}

#[test]
fn controller_auth_room_creation_is_admitted_before_any_batched_authority_changes() {
    for auth_first in [false, true] {
        let mut runtime = ServerRuntime::new();
        hello(&mut runtime, "small", "room", json!({"uiMode":"GUI"})).unwrap();
        hello(&mut runtime, "large", "room", large_features()).unwrap();
        let before = list(&mut runtime, "small");
        let password = "AB-123-456";
        let generated = runtime
            .handle_line(
                "large",
                &json!({"Set":{"controllerAuth":{
                    "room":"x".repeat(20_000),"password":password
                }}})
                .to_string(),
            )
            .unwrap();
        let ProtocolMessage::Set(response) = decode_message_line(&generated[0]).unwrap() else {
            panic!("controlled room response expected");
        };
        let room = response.set.new_controlled_room.unwrap().room_name.unwrap();
        let auth = json!({"room":room,"password":password}).to_string();
        let update = if auth_first {
            format!(r#"{{"Set":{{"controllerAuth":{auth},"file":{{"name":"new.mkv"}}}}}}"#)
        } else {
            format!(r#"{{"Set":{{"file":{{"name":"new.mkv"}},"controllerAuth":{auth}}}}}"#)
        };
        assert!(runtime.handle_line("large", &update).is_err());
        assert!(runtime.sessions["large"].file.is_none());
        assert!(!runtime.room_controllers.contains_key(&room));
        assert!(!runtime.room_playlists.contains_key(&room));
        assert!(!runtime.room_playback_states.contains_key(&room));
        assert_eq!(list(&mut runtime, "small"), before);

        // An ordinary controlled room still grants authority and remains visible.
        let room = runtime
            .room_password_provider
            .controlled_room_name_for("normal", password);
        runtime
            .handle_line(
                "large",
                &json!({"Set":{"controllerAuth":{
                    "room":room,"password":password
                }}})
                .to_string(),
            )
            .unwrap();
        assert!(runtime.room_controllers[&room].contains("large"));
        assert!(list(&mut runtime, "small").contains_key(&room));
    }
}

#[test]
fn configured_peer_byte_budget_rejects_growth_before_committing_reliable_state() {
    let mut runtime = ServerRuntime::new();
    let limits = crate::ServerResourceLimits {
        queued_bytes_per_peer: 1024,
        ..Default::default()
    };
    runtime.set_resource_limits(limits).unwrap();
    for id in ["alice", "bob"] {
        hello(&mut runtime, id, "room", large_features()).unwrap();
    }
    let before = list(&mut runtime, "bob");
    let update = json!({"Set":{"file":{"name":"new.mkv","extension":"x".repeat(2000)}}});
    assert!(runtime.handle_line("alice", &update.to_string()).is_err());
    assert!(runtime.sessions["alice"].file.is_none());
    assert_eq!(list(&mut runtime, "bob"), before);

    let resources = crate::resources::NetworkResources::new(limits);
    let accepted = runtime
        .handle_line_fanout("alice", r#"{"Set":{"file":{"name":"ok.mkv"}}}"#)
        .unwrap();
    assert_eq!(accepted.len(), 2);
    for outbound in accepted {
        let budget = resources.peer_budget();
        assert!(
            budget.reserve(outbound.line.len() + 2).is_some(),
            "accepted reliable frame must fit with CRLF"
        );
    }
    assert_eq!(
        list(&mut runtime, "bob")["room"]["alice"]
            .file
            .as_ref()
            .unwrap()["name"],
        "ok.mkv"
    );
}

#[test]
fn reducing_peer_byte_budget_rejects_incompatible_existing_state() {
    let mut runtime = ServerRuntime::new();
    hello(&mut runtime, "alice", "room", large_features()).unwrap();
    runtime
        .handle_line(
            "alice",
            &json!({"Set":{"file":{"name":"new.mkv","extension":"x".repeat(2000)}}}).to_string(),
        )
        .unwrap();
    let before = list(&mut runtime, "alice");
    let original = runtime.resource_limits;
    assert!(
        runtime
            .set_resource_limits(crate::ServerResourceLimits {
                queued_bytes_per_peer: 1024,
                ..original
            })
            .is_err()
    );
    assert_eq!(runtime.resource_limits, original);
    assert_eq!(list(&mut runtime, "alice"), before);
}

fn list(
    runtime: &mut ServerRuntime,
    id: &str,
) -> BTreeMap<String, BTreeMap<String, ListUserEntry>> {
    let output = runtime.handle_line(id, r#"{"List":null}"#).unwrap();
    assert!(
        output
            .iter()
            .all(|line| line.len() <= runtime.recipient_frame_limit(id))
    );
    let ProtocolMessage::List(message) = decode_message_line(&output[0]).unwrap() else {
        panic!("List expected");
    };
    let ListPayload::Rooms(rooms) = message.list else {
        panic!("snapshot expected");
    };
    rooms
}

#[test]
fn aggregate_extension_growth_is_rejected_before_file_authority_changes() {
    let mut runtime = ServerRuntime::new();
    for id in ["alice", "bob", "reader"] {
        hello(&mut runtime, id, "room", large_features()).unwrap();
    }
    let update = json!({"Set":{"file":{"name":"large.mkv","duration":100.0,"unknownExtension":"x".repeat(300_000)}}}).to_string();
    runtime.handle_line("alice", &update).unwrap();
    assert!(runtime.handle_line("bob", &update).is_err());
    assert!(
        runtime.sessions["bob"].file.is_none(),
        "failed growth committed a file"
    );
    let rooms = list(&mut runtime, "reader");
    assert_eq!(rooms["room"].len(), 3);
    assert_eq!(
        rooms["room"]["alice"].file.as_ref().unwrap()["unknownExtension"]
            .as_str()
            .unwrap()
            .len(),
        300_000
    );
    assert!(
        rooms["room"]["bob"]
            .file
            .as_ref()
            .unwrap()
            .get("unknownExtension")
            .is_none()
    );
}

#[test]
fn late_legacy_join_and_capability_downgrade_preserve_existing_sessions() {
    let mut runtime = ServerRuntime::new();
    hello(&mut runtime, "alice", "room", large_features()).unwrap();
    runtime
        .handle_line(
            "alice",
            &json!({"Set":{"file":{"name":"large.mkv","unknownExtension":"x".repeat(70_000)}}})
                .to_string(),
        )
        .unwrap();
    assert!(hello(&mut runtime, "legacy", "room", json!({})).is_err());
    assert!(!runtime.sessions.contains_key("legacy"));
    assert!(
        runtime
            .handle_line("alice", r#"{"Set":{"features":{}}}"#)
            .is_err()
    );
    assert!(
        runtime.sessions["alice"]
            .capabilities
            .large_protocol_frames_v1
    );
    hello(&mut runtime, "bob", "room", large_features()).unwrap();
    runtime
        .handle_line(
            "bob",
            &json!({"Set":{"file":{"name":"large.mkv","unknownExtension":"y".repeat(70_000)}}})
                .to_string(),
        )
        .unwrap();
    assert!(hello(&mut runtime, "alice", "room", json!({})).is_err());
    assert!(
        runtime.sessions["alice"].file.is_some(),
        "failed reconnect must preserve old authority"
    );
    assert_eq!(list(&mut runtime, "alice")["room"].len(), 2);
}

#[test]
fn isolated_rooms_do_not_impose_unrelated_small_peer_limits() {
    let mut runtime = ServerRuntime::new();
    runtime.set_isolate_rooms(true);
    hello(&mut runtime, "legacy", "small", json!({})).unwrap();
    hello(&mut runtime, "large", "large", large_features()).unwrap();
    runtime
        .handle_line(
            "large",
            &json!({"Set":{"file":{"name":"large.mkv","unknownExtension":"x".repeat(70_000)}}})
                .to_string(),
        )
        .unwrap();
    assert!(
        runtime
            .handle_line("large", r#"{"Set":{"room":{"name":"small"}}}"#)
            .is_err()
    );
    assert_eq!(runtime.sessions["large"].room, "large");
    assert_eq!(list(&mut runtime, "legacy")["small"].len(), 1);
}

#[test]
fn permanent_room_projection_uses_bounded_unique_dummy_identities() {
    let mut runtime = ServerRuntime::new();
    runtime.set_permanent_rooms((0..1024).map(|index| format!("room-{index:04}")));
    hello(&mut runtime, "large", "occupied", large_features()).unwrap();
    let rooms = list(&mut runtime, "large");
    assert_eq!(rooms.len(), 1025);
    let identities: BTreeSet<_> = rooms
        .iter()
        .filter(|(room, _)| room.as_str() != "occupied")
        .map(|(_, users)| users.keys().next().unwrap().clone())
        .collect();
    assert_eq!(identities.len(), 1024);
    assert!(
        identities
            .iter()
            .all(|name| name.len() <= 12 && name.chars().all(char::is_whitespace))
    );
    assert!(
        hello(
            &mut runtime,
            "legacy-gui",
            "occupied",
            json!({"uiMode":"GUI"})
        )
        .is_err()
    );
    assert_eq!(list(&mut runtime, "large").len(), 1025);
}

#[test]
fn optional_discovery_metadata_compacts_without_hiding_roster_members() {
    let mut runtime = ServerRuntime::new();
    let signature: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/media-match/maximum-valid-media-signature.json"
    ))
    .unwrap();
    sorotte_media_match::media_match_wire_signature_from_value(&signature)
        .expect("signature must satisfy actual Media Match validation");
    let update =
        json!({"Set":{"file":{"name":"episode.mkv","duration":100.0,"mediaMatch":signature}}})
            .to_string();
    for index in 0..57 {
        let id = format!("provider-{index}");
        hello(&mut runtime, &id, "room", large_features()).unwrap();
        runtime.handle_line(&id, &update).unwrap();
    }
    hello(&mut runtime, "reader", "room", large_features()).unwrap();
    let rooms = list(&mut runtime, "reader");
    assert_eq!(rooms["room"].len(), 58);
    assert!(rooms["room"].values().all(|entry| {
        entry
            .file
            .as_ref()
            .is_none_or(|file| file.get("mediaMatch").is_none())
    }));
}

#[test]
fn configuration_visibility_and_permanent_rooms_preserve_state_on_frame_failure() {
    let mut runtime = ServerRuntime::new();
    runtime.set_isolate_rooms(true);
    hello(&mut runtime, "legacy", "small", json!({"uiMode":"GUI"})).unwrap();
    hello(&mut runtime, "large", "large", large_features()).unwrap();
    runtime
        .handle_line(
            "large",
            &json!({"Set":{"file":{"name":"video", "extension":"x".repeat(70_000)}}}).to_string(),
        )
        .unwrap();
    assert!(runtime.try_set_isolate_rooms(false).is_err());
    assert!(runtime.isolate_rooms);
    assert!(
        runtime
            .try_set_permanent_rooms((0..1024).map(|i| format!("permanent-{i:04}")))
            .is_err()
    );
    assert_eq!(list(&mut runtime, "legacy").len(), 1);
    assert_eq!(runtime.sessions.len(), 2);
}

#[test]
fn playlist_and_capability_batch_is_validated_before_either_order_commits() {
    for features_first in [false, true] {
        let mut runtime = ServerRuntime::new();
        hello(&mut runtime, "alice", "room", large_features()).unwrap();
        runtime
            .handle_line(
                "alice",
                r#"{"Set":{"playlistChange":{"files":["old.mkv"]}}}"#,
            )
            .unwrap();
        let playlist = json!({"files":["x".repeat(20_000)]}).to_string();
        let message = if features_first {
            format!(r#"{{"Set":{{"features":{{}},"playlistChange":{playlist}}}}}"#)
        } else {
            format!(r#"{{"Set":{{"playlistChange":{playlist},"features":{{}}}}}}"#)
        };
        assert!(runtime.handle_line("alice", &message).is_err());
        assert!(
            runtime.sessions["alice"]
                .capabilities
                .large_protocol_frames_v1
        );
        assert_eq!(runtime.room_playlist_state("room").files, ["old.mkv"]);
    }
}

#[test]
fn playlist_fanout_budget_is_checked_before_replacing_shared_files() {
    let mut runtime = ServerRuntime::new();
    runtime
        .set_resource_limits(crate::ServerResourceLimits {
            queued_bytes_per_peer: 4096,
            queued_bytes_total: 8192,
            ..Default::default()
        })
        .unwrap();
    for id in ["alice", "bob", "reader"] {
        hello(&mut runtime, id, "room", large_features()).unwrap();
    }
    let message = json!({"Set":{"playlistChange":{"files":["x".repeat(1500)]}}}).to_string();
    assert!(runtime.handle_line("alice", &message).is_err());
    assert!(runtime.room_playlist_state("room").files.is_empty());
}

#[test]
fn retained_barrier_cohort_is_counted_when_new_members_join_after_disconnects() {
    let mut runtime = ServerRuntime::new();
    runtime.set_time_now_override_seconds(Some(100.0));
    let capabilities = json!({"sorottePlaybackBarrierV1":true});
    let mut peers = Vec::new();
    for index in 0..64 {
        let id = format!("participant-{index:02}");
        if hello(&mut runtime, &id, "room", capabilities.clone()).is_err() {
            break;
        }
        peers.push(id);
    }
    assert!((2..64).contains(&peers.len()));
    runtime
        .handle_line(
            &peers[0],
            &json!({"Set":{"sorottePlaybackBarrierV1":{"prepare":{
                "mediaGeneration":0,"requestNonce":1,"logicalMediaId":"\u{1}".repeat(2048),
                "targetPosition":0.0,"policy":"allEligible","timeoutMs":30_000
            }}}})
            .to_string(),
        )
        .unwrap();
    assert_eq!(
        runtime.room_playback_barriers["room"].participants.len(),
        peers.len()
    );
    for peer in &peers[1..] {
        runtime.handle_transport_disconnect_fanout(peer).unwrap();
    }
    assert_eq!(runtime.sessions.len(), 1);
    assert!(hello(&mut runtime, "new-participant", "room", capabilities).is_err());
    assert_eq!(runtime.sessions.len(), 1);
    for message in runtime.playback_barrier_snapshot_for_client("room", &peers[0]) {
        let encoded = encode_message_line(&message.message).unwrap();
        assert!(encoded.len() <= sorotte_protocol::DEFAULT_MAX_PROTOCOL_LINE_BYTES);
    }
}
