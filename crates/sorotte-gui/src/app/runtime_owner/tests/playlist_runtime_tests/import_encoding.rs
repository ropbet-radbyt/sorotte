//! Playlist encoding regression coverage through the ordinary import request.
use super::*;

fn imported_first_file(extension: &str, with_bom: bool, first_is_url: bool) -> bool {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first.mkv");
    let second = fixture.path().join("second.mkv");
    std::fs::write(&first, b"first fixture").unwrap();
    std::fs::write(&second, b"second fixture").unwrap();
    let playlist = fixture.path().join(format!("room.{extension}"));
    let prefix = if with_bom { "\u{feff}" } else { "" };
    let first_target = if first_is_url {
        "https://media.example.test/first.mp4".to_owned()
    } else {
        first.display().to_string()
    };
    std::fs::write(
        &playlist,
        format!("{prefix}{first_target}\n{}\n", second.display()),
    )
    .unwrap();
    let mut owner = GuiPersistedConfigRuntimeOwner::with_config_path(None);
    owner.player = Some(GuiOwnedPlayer::Test(GuiTestPlayerAdapter::default()));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let mut shell = SorotteGuiShellAppState::from_stored_settings(&StoredClientSettings {
        media_search_directories: Some(vec![fixture.path().to_string_lossy().into_owned()]),
        media_matching_plugin_enabled: Some(false),
        plex_plugin_enabled: Some(false),
        only_switch_to_trusted_domains: Some(false),
        check_for_updates_automatically: Some(false),
        public_servers: Some(vec![]),
        ..Default::default()
    });
    handle.push_request(GuiRuntimeRequest::ImportSharedPlaylistFile {
        path: playlist.to_string_lossy().into_owned(),
        shuffled: false,
    });
    for _ in 0..20 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut shell);
    }
    let loaded = owner
        .player_local_file
        .as_ref()
        .and_then(|file| file.path.clone());
    let loaded_first = loaded.as_deref().is_some_and(|path| {
        if first_is_url {
            path == first_target
        } else {
            std::fs::canonicalize(path).unwrap() == std::fs::canonicalize(&first).unwrap()
        }
    });
    eprintln!(
        "extension={extension}, BOM={with_bom}, URL={first_is_url}, imported={:?}, first loaded={loaded:?}",
        shell.current_shared_playlist_entries()
    );
    assert_eq!(shell.main_window.playlist.len(), 2);
    assert!(
        owner.pending_attached_media_resolution.is_none(),
        "the first target must not merely be awaiting a local media scan"
    );
    if first_is_url {
        assert_eq!(shell.current_shared_playlist_entries()[0], first_target);
        assert!(reqwest::Url::parse(&shell.current_shared_playlist_entries()[0]).is_ok());
    }

    // A following valid import must recover in the same owner without a restart.
    let next_playlist = fixture.path().join("next.m3u");
    std::fs::write(&next_playlist, format!("{}\n", second.display())).unwrap();
    handle.push_request(GuiRuntimeRequest::ImportSharedPlaylistFile {
        path: next_playlist.to_string_lossy().into_owned(),
        shuffled: false,
    });
    for _ in 0..20 {
        pump_and_apply_runtime_owner_actions(&mut owner, &handle, &mut shell);
    }
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref())
            .map(|path| std::fs::canonicalize(path).unwrap()),
        Some(std::fs::canonicalize(&second).unwrap())
    );
    loaded_first
}

#[test]
fn text_playlist_bom_opens_first_file() {
    assert!(
        imported_first_file("txt", true, false),
        "UTF-8 BOM must not become part of the first playlist target"
    );
}

#[test]
fn plain_text_playlist_control() {
    assert!(imported_first_file("txt", false, false));
}

#[test]
fn m3u_bom_control() {
    assert!(imported_first_file("m3u", true, false));
}

#[test]
fn text_playlist_bom_opens_first_url() {
    assert!(
        imported_first_file("txt", true, true),
        "UTF-8 BOM must not corrupt the first imported URL"
    );
}

#[test]
fn plain_text_url_control() {
    assert!(imported_first_file("txt", false, true));
}

#[test]
fn m3u_bom_url_control() {
    assert!(imported_first_file("m3u", true, true));
}
