use sorotte_client_core::{ClientUserView, PeerCapabilities, ServerCapabilities};

#[test]
fn downstream_legacy_struct_literals_remain_source_compatible() {
    let capabilities = PeerCapabilities {
        shared_playlists: false,
        chat: true,
        feature_list: true,
        readiness: true,
        managed_rooms: false,
        persistent_rooms: false,
        media_match: true,
        plex_playlist_uris: false,
        remote_readiness: false,
        playback_barrier_v1: true,
        readiness_v2: true,
        ui_mode: None,
    };
    let user = ClientUserView {
        room: Some("room".to_owned()),
        ready: Some(false),
        file: None,
        capabilities: Some(capabilities),
        controller: false,
    };
    let server = ServerCapabilities {
        chat: true,
        readiness: true,
        remote_readiness: false,
        shared_playlists: true,
        managed_rooms: false,
        media_match: true,
        plex_playlist_uris: false,
        playback_barrier_v1: true,
        readiness_v2: true,
        persistent_rooms: false,
        max_username_length: 16,
        max_room_name_length: 35,
        max_filename_length: 250,
    };

    assert_eq!(user.room.as_deref(), Some("room"));
    assert!(server.chat);
}
