use super::*;

#[test]
fn protocol_lines_for_startup_playlist_load_from_file_legacy_compatible_emits_playlist_change_then_index()
 {
    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir =
        std::env::temp_dir().join(format!("syncplay-cli-playlist-load-test-{unique_suffix}"));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let playlist_path = temp_dir.join("startup-playlist.txt");
    std::fs::write(&playlist_path, "episode1.mkv\nepisode2.mkv\n")
        .expect("playlist file should write");

    let lines = protocol_lines_for_startup_playlist_load_from_file_legacy_compatible(
        playlist_path.as_path(),
    )
    .expect("playlist file should load");
    assert_eq!(lines.len(), 2);

    let first = decode_message_line(&lines[0]).expect("playlist change line should decode");
    let second = decode_message_line(&lines[1]).expect("playlist index line should decode");
    let ProtocolMessage::Set(first_set) = first else {
        panic!("first startup playlist line should be Set");
    };
    let ProtocolMessage::Set(second_set) = second else {
        panic!("second startup playlist line should be Set");
    };
    let playlist_change = first_set
        .set
        .playlist_change
        .expect("first set should contain playlistChange");
    assert_eq!(
        playlist_change.files,
        vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()]
    );
    assert!(
        first_set.set.playlist_index.is_none(),
        "playlistChange message should not also contain playlistIndex"
    );
    let playlist_index = second_set
        .set
        .playlist_index
        .expect("second set should contain playlistIndex");
    assert_eq!(playlist_index.index, 0);
    assert!(
        second_set.set.playlist_change.is_none(),
        "playlistIndex message should not also contain playlistChange"
    );

    let _ = std::fs::remove_file(&playlist_path);
    let _ = std::fs::remove_dir(&temp_dir);
}

#[tokio::test]
async fn connected_client_session_sends_startup_playlist_from_legacy_file_after_server_hello() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener
        .local_addr()
        .expect("listener should have local addr");

    let unique_suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be monotonic enough for test")
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "syncplay-cli-startup-playlist-session-{unique_suffix}"
    ));
    std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
    let playlist_path = temp_dir.join("startup-playlist.txt");
    std::fs::write(&playlist_path, "episode1.mkv\nepisode2.mkv\n")
        .expect("playlist file should write");

    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("server should accept");
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();

        let hello_line = lines
            .next_line()
            .await
            .expect("hello line read should succeed")
            .expect("hello line should be present");
        assert!(hello_line.contains("\"Hello\""));
        writer
                .write_all(
                    br#"{"Hello":{"username":"cli-user","room":{"name":"cli-room"},"version":"1.2.255","features":{"sharedPlaylists":true}}}
"#,
                )
                .await
                .expect("server hello write should succeed");

        let mut saw_playlist_change = None;
        let mut saw_playlist_index = None;
        for _ in 0..6 {
            let Some(line) = tokio::time::timeout(Duration::from_secs(1), lines.next_line())
                .await
                .expect("client line should not timeout")
                .expect("client line read should succeed")
            else {
                break;
            };
            let message = decode_message_line(&line).expect("line should decode");
            let ProtocolMessage::Set(set_payload) = message else {
                continue;
            };
            if let Some(change) = set_payload.set.playlist_change {
                saw_playlist_change = Some(change.files);
            }
            if let Some(index) = set_payload.set.playlist_index {
                saw_playlist_index = Some(index.index);
            }
            if saw_playlist_change.is_some() && saw_playlist_index.is_some() {
                break;
            }
        }

        assert_eq!(
            saw_playlist_change,
            Some(vec!["episode1.mkv".to_owned(), "episode2.mkv".to_owned()])
        );
        assert_eq!(saw_playlist_index, Some(0));
    });

    let config = test_client_loop_config_with_addr(addr);
    let mut runtime = create_client_runtime(&config);
    let stream = TcpStream::connect(addr)
        .await
        .expect("client should connect to test listener");
    let mut notification_sink = ignore_autoplay_notification;
    let mut file_difference_sink = ignore_file_difference_notification;
    let mut startup_playlist = Some(playlist_path.to_string_lossy().into_owned());

    let exit = run_connected_client_session_with_legacy_startup_overrides(
        stream,
        &mut runtime,
        &config,
        None,
        &mut startup_playlist,
        None,
        &mut notification_sink,
        &mut file_difference_sink,
    )
    .await
    .expect("connected session should run");
    assert!(
        matches!(
            exit,
            ConnectedSessionExit::TransportClosed | ConnectedSessionExit::RuntimeWindowElapsed
        ),
        "session should either observe peer close or runtime window exit"
    );
    assert!(
        startup_playlist.is_none(),
        "startup playlist flag should be consumed after server hello"
    );
    server_task.await.expect("server task should join");

    let _ = std::fs::remove_file(&playlist_path);
    let _ = std::fs::remove_dir(&temp_dir);
}
