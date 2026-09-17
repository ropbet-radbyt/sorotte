use super::*;
use crate::app::runtime_owner::media_match_lookup::FingerprintLookup;
use sorotte_media_match::{
    AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION, MediaExtractionSettings, MediaFileIdentity,
    MediaFingerprintRecord, MediaIndexService, media_match_wire_value_from_records,
};
use std::{
    fs,
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy, Debug)]
enum FileTransition {
    Unchanged,
    SameSizeEdit,
    SizeChange,
    SameSizeEditReindexed,
    TimestampDrift,
    TimestampDriftReindexed,
    GeneratedAudioEdit,
    GeneratedTimestampDrift,
}

fn write_seeded_pcm_wave(path: &std::path::Path, mut seed: u32) {
    use std::io::Write;
    const RATE: u32 = 8000;
    const SAMPLES: u32 = RATE * 120;
    let data_bytes = SAMPLES * 2;
    let mut file = fs::File::create(path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&RATE.to_le_bytes()).unwrap();
    file.write_all(&(RATE * 2).to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_bytes.to_le_bytes()).unwrap();
    let mut pcm = Vec::with_capacity(data_bytes as usize);
    for _ in 0..SAMPLES {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let sample = ((seed >> 16) as i16) / 4;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    file.write_all(&pcm).unwrap();
}

fn write_generated_media(path: &std::path::Path, seed: u32) {
    // Match the GUI's supported media inventory: place deterministic PCM audio
    // in a Matroska container, retaining equal byte lengths across editions.
    let wave = path.with_extension("wav");
    write_seeded_pcm_wave(&wave, seed);
    let output = std::process::Command::new(
        std::env::var_os("SOROTTE_MEDIA_MATCH_FFMPEG").expect("set cached ffmpeg path"),
    )
    .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
    .arg(&wave)
    .args([
        "-map",
        "0:a:0",
        "-c:a",
        "copy",
        "-fflags",
        "+bitexact",
        "-map_metadata",
        "-1",
        "-f",
        "matroska",
    ])
    .arg(path)
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "local media mux failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn extract_generated_audio(path: &std::path::Path) -> MediaFingerprintRecord {
    let tools = sorotte_media_match::MediaMatchToolPaths {
        ffmpeg: std::env::var_os("SOROTTE_MEDIA_MATCH_FFMPEG")
            .expect("set cached ffmpeg path")
            .into(),
        ffprobe: std::env::var_os("SOROTTE_MEDIA_MATCH_FFPROBE")
            .expect("set cached ffprobe path")
            .into(),
    };
    let extracted = sorotte_media_match::fingerprint_media_file_with_report(
        path,
        &tools,
        &MediaExtractionSettings::sampled_fast_audio_index_v3(),
        None,
    )
    .unwrap();
    assert!(extracted.record.audio_error.is_none());
    assert!(!extracted.record.audio_anchors.is_empty());
    assert_eq!(extracted.record.duration_seconds, Some(120.0));
    extracted.record
}

fn record_for_path(path: &std::path::Path, bucket_base: u32) -> MediaFingerprintRecord {
    let metadata = fs::metadata(path).unwrap();
    MediaFingerprintRecord {
        identity: MediaFileIdentity::new(
            path,
            metadata
                .modified()
                .unwrap()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            metadata.len(),
        ),
        algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
        extraction_settings: MediaExtractionSettings::sampled_fast_audio_index_v3(),
        duration_seconds: Some(120.0),
        container_fingerprint: format!("edition-{bucket_base}"),
        audio_anchors: (0..48)
            .map(|index| AudioAnchor {
                bucket: bucket_base + index,
                t_ms: index * 2000,
                weight: 10,
            })
            .collect(),
        audio_error: None,
    }
}

fn publish_file_after_transition(
    transition: FileTransition,
) -> (Option<serde_json::Value>, serde_json::Value) {
    let fixture = tempfile::tempdir().unwrap();
    let generated = matches!(
        transition,
        FileTransition::GeneratedAudioEdit | FileTransition::GeneratedTimestampDrift
    );
    let media_path = fixture.path().join("episode.mkv");
    let first_modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_750_000_000);
    if generated {
        write_generated_media(&media_path, 20260731);
    } else {
        fs::write(&media_path, vec![1u8; 4096]).unwrap();
    }
    fs::File::options()
        .write(true)
        .open(&media_path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(first_modified))
        .unwrap();
    let original = if generated {
        extract_generated_audio(&media_path)
    } else {
        record_for_path(&media_path, 1000)
    };
    let original_wire =
        media_match_wire_value_from_records(std::slice::from_ref(&original)).unwrap();
    let index_service = MediaIndexService::new(fixture.path().join("cache/media-match"));
    index_service
        .open()
        .unwrap()
        .save_record(&original, None)
        .unwrap();
    let (mut owner, _transport) =
        GuiPersistedConfigRuntimeOwner::with_config_path(Some(fixture.path().join("sorotte.ini")))
            .with_recording_chat_session_runtime("alice", "room1")
            .unwrap();
    owner.session.as_mut().unwrap().apply_message_json(r#"{"Hello":{"username":"alice","room":{"name":"room1"},"version":"1.7.5","features":{"mediaMatch":true}}}"#).unwrap();
    owner
        .session
        .as_mut()
        .unwrap()
        .deliver_outbound_protocol_lines()
        .unwrap();
    assert!(
        matches!(
            owner
                .media_match_record_lookup
                .wait_for_test(fixture.path(), media_path.to_str().unwrap()),
            FingerprintLookup::Present(_)
        ),
        "original valid fingerprint should be available"
    );

    if !matches!(transition, FileTransition::Unchanged) {
        let bytes = if matches!(transition, FileTransition::SizeChange) {
            4097
        } else {
            4096
        };
        if matches!(transition, FileTransition::GeneratedAudioEdit) {
            write_generated_media(&media_path, 20260917);
        } else if !matches!(
            transition,
            FileTransition::TimestampDrift
                | FileTransition::TimestampDriftReindexed
                | FileTransition::GeneratedTimestampDrift
        ) {
            fs::write(&media_path, vec![2u8; bytes]).unwrap();
        }
        fs::File::options()
            .write(true)
            .open(&media_path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(first_modified + Duration::from_secs(10)))
            .unwrap();
    }
    let mut current = if generated {
        extract_generated_audio(&media_path)
    } else {
        record_for_path(&media_path, 5000)
    };
    if generated {
        assert_eq!(
            current.identity.size_bytes, original.identity.size_bytes,
            "generated versions must have identical length"
        );
        let current_wire =
            media_match_wire_value_from_records(std::slice::from_ref(&current)).unwrap();
        if matches!(transition, FileTransition::GeneratedAudioEdit) {
            assert_ne!(current.audio_anchors, original.audio_anchors);
            assert_ne!(current_wire, original_wire);
        } else {
            assert_eq!(current.audio_anchors, original.audio_anchors);
            assert_eq!(current_wire, original_wire);
        }
    }
    if !matches!(transition, FileTransition::Unchanged) {
        assert_ne!(
            current.identity.modified_unix_millis,
            original.identity.modified_unix_millis
        );
        assert!(!original.valid_for(
            &current.identity.normalized_path,
            current.identity.modified_unix_millis,
            current.identity.size_bytes,
            MEDIA_MATCH_ALGORITHM_VERSION,
            &current.extraction_settings
        ));
    }
    if matches!(transition, FileTransition::TimestampDriftReindexed) {
        current.audio_anchors.clone_from(&original.audio_anchors);
    }
    let expected_wire = if matches!(
        transition,
        FileTransition::SameSizeEditReindexed | FileTransition::TimestampDriftReindexed
    ) {
        index_service
            .open()
            .unwrap()
            .save_record(&current, None)
            .unwrap();
        media_match_wire_value_from_records(std::slice::from_ref(&current)).unwrap()
    } else {
        original_wire.clone()
    };
    let _ = owner
        .media_match_record_lookup
        .wait_for_test(fixture.path(), media_path.to_str().unwrap());
    owner.player_local_file = Some(
        sorotte_player_api::LocalFileUpdate::new(media_path.file_name().unwrap().to_string_lossy())
            .with_path(media_path.to_string_lossy().into_owned())
            .with_duration_seconds(120.0)
            .with_size_bytes(current.identity.size_bytes),
    );
    let mut state =
        crate::app::runtime_state::GuiRuntimeState::from_stored_settings(&StoredClientSettings {
            username: Some("alice".to_owned()),
            room: Some("room1".to_owned()),
            shared_playlist_enabled: Some(true),
            media_matching_plugin_enabled: Some(true),
            media_match_fingerprinting_enabled: Some(true),
            media_match_wire_sharing_enabled: Some(true),
            ..StoredClientSettings::default()
        });
    owner
        .sync_detached_session_preferences_and_player_state(&state)
        .unwrap();
    let outbound = owner
        .session
        .as_mut()
        .unwrap()
        .deliver_outbound_protocol_lines()
        .unwrap();
    let file_payload = outbound
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|message| message.get("Set")?.get("file").cloned())
        .expect("production owner must publish the observed local file");
    let advertised = file_payload.get("mediaMatch").cloned();
    if generated {
        assert!(
            advertised.is_none(),
            "changed file identity must withhold the old signature"
        );
        revalidate_selected_generated_media(
            &mut owner,
            &mut state,
            fixture.path(),
            &media_path,
            &current,
        );
    }
    (advertised, expected_wire)
}

fn revalidate_selected_generated_media(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    state: &mut crate::app::runtime_state::GuiRuntimeState,
    root: &std::path::Path,
    media_path: &std::path::Path,
    expected_record: &MediaFingerprintRecord,
) {
    use crate::app::media_match_support::{MediaMatchTool, managed_media_match_tool_path};

    for (tool, variable) in [
        (MediaMatchTool::Ffmpeg, "SOROTTE_MEDIA_MATCH_FFMPEG"),
        (MediaMatchTool::Ffprobe, "SOROTTE_MEDIA_MATCH_FFPROBE"),
    ] {
        let source = std::env::var_os(variable).expect("explicit tool path is required");
        let destination = managed_media_match_tool_path(root, tool);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        if fs::hard_link(&source, &destination).is_err() {
            fs::copy(source, destination).unwrap();
        }
    }
    let path = media_path.to_string_lossy().into_owned();
    let label = media_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    state.apply_shared_playlist_entries(vec![label], Some(0), false);
    state.playlist.main_window.active_playlist_index = Some(0);
    owner.active_shared_playlist_index = Some(0);
    owner.remember_local_shared_playlist_media_match_signature_path(&path);
    let row_id = state.playlist.main_window.playlist[0].entry_id;
    assert!(matches!(
        owner.media_match_record_lookup.wait_for_test(root, &path),
        FingerprintLookup::Missing
    ));
    let handle = GuiQueuedRuntimeBridgeHandle::default();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    // Tool readiness is discovered asynchronously in the ordinary runtime
    // pump. Keep ticking that probe and the scheduler instead of supplying a
    // synthetic Healthy snapshot that would bypass the public worker path.
    while owner.media_match_background_worker_rx.is_none() {
        owner.pump_media_match_tool_worker(&handle, state);
        owner.maybe_queue_media_match_exact_playlist_signature(&handle, state);
        assert!(
            std::time::Instant::now() < deadline,
            "normal scheduler must queue a fresh exact signature: {:?}",
            owner.media_match_runtime_snapshot
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    while owner.media_match_background_worker_rx.is_some() {
        owner.pump_media_match_background_worker(&handle, state);
        assert!(
            std::time::Instant::now() < deadline,
            "signature worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let value = match owner.media_match_record_lookup.wait_for_test(root, &path) {
        FingerprintLookup::Present(value) => value,
        FingerprintLookup::Failed(error) => panic!("background extraction lookup failed: {error}"),
        FingerprintLookup::Missing => panic!(
            "background extraction record is missing: {:?}",
            owner.media_match_runtime_snapshot
        ),
        FingerprintLookup::Pending => unreachable!(),
    };
    assert_eq!(value.record.identity, expected_record.identity);
    assert_eq!(value.record.audio_anchors, expected_record.audio_anchors);
    assert_eq!(state.playlist.main_window.active_playlist_index, Some(0));
    assert_eq!(state.playlist.main_window.playlist[0].entry_id, row_id);
    assert_eq!(
        owner
            .player_local_file
            .as_ref()
            .and_then(|file| file.path.as_deref()),
        Some(path.as_str())
    );

    owner
        .sync_detached_session_preferences_and_player_state(state)
        .unwrap();
    let outbound = owner
        .session
        .as_mut()
        .unwrap()
        .deliver_outbound_protocol_lines()
        .unwrap();
    let expected_wire =
        media_match_wire_value_from_records(std::slice::from_ref(expected_record)).unwrap();
    assert!(
        outbound
            .iter()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .any(|message| message
                .get("Set")
                .and_then(|set| set.get("file"))
                .and_then(|file| file.get("mediaMatch"))
                == Some(&expected_wire)),
        "revalidation must publish the newly extracted audio signature"
    );
    owner.maybe_queue_media_match_exact_playlist_signature(&handle, state);
    assert!(
        owner.media_match_background_worker_rx.is_none(),
        "unchanged revalidated identity must remain warm"
    );
}

#[test]
fn media_fingerprint_same_size_edit_does_not_advertise_old_audio() {
    let (advertised, _) = publish_file_after_transition(FileTransition::SameSizeEdit);
    assert!(
        advertised.is_none(),
        "after a same-size file edit the GUI must await a fresh fingerprint before sharing media identity"
    );
}

#[test]
fn media_fingerprint_unchanged_sharing_control() {
    let (advertised, expected) = publish_file_after_transition(FileTransition::Unchanged);
    assert_eq!(advertised, Some(expected));
}

#[test]
fn media_fingerprint_changed_size_withholds_sharing_control() {
    let (advertised, _) = publish_file_after_transition(FileTransition::SizeChange);
    assert!(advertised.is_none());
}

#[test]
fn media_fingerprint_reindex_shares_new_audio_control() {
    let (advertised, expected) =
        publish_file_after_transition(FileTransition::SameSizeEditReindexed);
    assert_eq!(advertised, Some(expected));
}

#[test]
fn media_fingerprint_timestamp_drift_awaits_revalidation() {
    let (advertised, _) = publish_file_after_transition(FileTransition::TimestampDrift);
    assert!(advertised.is_none());
}

#[test]
fn media_fingerprint_timestamp_drift_revalidation_restores_signature() {
    let (advertised, expected) =
        publish_file_after_transition(FileTransition::TimestampDriftReindexed);
    assert_eq!(
        advertised,
        Some(expected),
        "unchanged audio should retain intended timestamp-drift compatibility"
    );
}

#[test]
#[ignore = "requires explicit local SOROTTE_MEDIA_MATCH_FFMPEG and SOROTTE_MEDIA_MATCH_FFPROBE paths"]
fn media_fingerprint_generated_equal_length_audio_replacement() {
    let (advertised, _) = publish_file_after_transition(FileTransition::GeneratedAudioEdit);
    assert!(
        advertised.is_none(),
        "equal-length replacement with a different extracted audio signature must not advertise the previous file's audio identity"
    );
}

#[test]
#[ignore = "requires explicit local SOROTTE_MEDIA_MATCH_FFMPEG and SOROTTE_MEDIA_MATCH_FFPROBE paths"]
fn media_fingerprint_generated_timestamp_drift_revalidates_same_audio() {
    let (advertised, _) = publish_file_after_transition(FileTransition::GeneratedTimestampDrift);
    assert!(advertised.is_none());
}
