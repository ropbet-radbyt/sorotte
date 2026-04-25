pub(super) const MOTD_TEMPLATE_SCENARIO: &str = "server_runtime_motd_template.jsonl";
pub(super) const MOTD_TEMPLATE_OUTDATED_SCENARIO: &str =
    "server_runtime_motd_template_outdated_client.jsonl";
pub(super) const MOTD_TEMPLATE_RUNTIME_AND_PROBE: &str = "Compat MOTD latest={latest_version}";
pub(super) const MOTD_TEMPLATE_LEGACY_FILE: &str = "Compat MOTD latest=$version";
pub(super) const MOTD_TEMPLATE_OUTDATED_EXPECTED: &str = "You are using Syncplay 1.2.255 but a newer version is available from https://syncplay.pl\nCompat MOTD latest=1.7.5";
pub(super) const PERSISTENT_ROOMS_NOTICE_SCENARIO: &str =
    "server_runtime_persistent_rooms_notice.jsonl";
pub(super) const PERSISTENT_ROOMS_LIFECYCLE_SCENARIO: &str =
    "server_runtime_persistent_rooms_lifecycle.jsonl";
pub(super) const PERSISTENT_ROOMS_TIMEOUT_LIST_UPDATES_SCENARIO: &str =
    "server_runtime_persistent_rooms_timeout_list_updates.jsonl";
pub(super) const PERMANENT_ROOMS_FILE_SCENARIO: &str = "server_runtime_permanent_rooms_file.jsonl";
pub(super) const CROSS_ROOM_PLAYLIST_SCOPING_SCENARIO: &str =
    "server_runtime_cross_room_playlist_scoping.jsonl";
pub(super) const PLAYLIST_ROOM_SWITCH_PEER_TRANSITION_SCOPING_SCENARIO: &str =
    "server_runtime_playlist_room_switch_peer_transition_scoping.jsonl";
pub(super) const PLAYLIST_DOUBLE_ROOM_SWITCH_SCOPING_SCENARIO: &str =
    "server_runtime_playlist_double_room_switch_scoping.jsonl";
pub(super) const PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_DESTINATION_UPDATE_ORDERING_SCENARIO: &str =
    "server_runtime_playlist_room_switch_snapshot_then_destination_update_ordering.jsonl";
pub(super) const PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_OLD_ROOM_UPDATE_ORDERING_SCENARIO: &str =
    "server_runtime_playlist_room_switch_snapshot_then_old_room_update_ordering.jsonl";
pub(super) const PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_OLD_THEN_DESTINATION_UPDATE_ORDERING_SCENARIO: &str =
    "server_runtime_playlist_room_switch_snapshot_then_old_then_destination_update_ordering.jsonl";
pub(super) const PLAYLIST_ROOM_SWITCH_SNAPSHOT_THEN_DESTINATION_THEN_OLD_UPDATE_ORDERING_SCENARIO: &str =
    "server_runtime_playlist_room_switch_snapshot_then_destination_then_old_update_ordering.jsonl";
pub(super) const CHAT_ROOM_SCOPING_SCENARIO: &str = "server_runtime_chat_room_scoping.jsonl";
pub(super) const CHAT_ROOM_SWITCH_SENDER_SCOPING_SCENARIO: &str =
    "server_runtime_chat_room_switch_sender_scoping.jsonl";
pub(super) const CHAT_ROOM_SWITCH_PEER_TRANSITION_SCOPING_SCENARIO: &str =
    "server_runtime_chat_room_switch_peer_transition_scoping.jsonl";
pub(super) const CHAT_ROOM_SWITCH_OBJECT_PAYLOAD_SCOPING_SCENARIO: &str =
    "server_runtime_chat_room_switch_object_payload_scoping.jsonl";
pub(super) const CHAT_DOUBLE_ROOM_SWITCH_SCOPING_SCENARIO: &str =
    "server_runtime_chat_double_room_switch_scoping.jsonl";
pub(super) const CHAT_USERNAME_NORMALIZATION_SCENARIO: &str =
    "server_runtime_chat_username_normalization.jsonl";
pub(super) const CHAT_PAYLOAD_NORMALIZATION_SCENARIO: &str =
    "server_runtime_chat_payload_normalization.jsonl";
pub(super) const PERMANENT_ROOMS_FILE_LIST: &[&str] = &["permanent-room"];
pub(super) const PERSISTENT_ROOMS_NOTICE: &str = "NOTICE: This server uses persistent rooms, which means that the playlist information is stored between playback sessions. If you want to create a room where information is not saved then put -temp at the end of the room name.";
