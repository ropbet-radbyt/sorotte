use super::*;

use crate::app::runtime_owner::player::media_resolution::{
    GuiMediaResolutionCandidate, GuiMediaResolutionPlan,
};
use crate::app::runtime_owner::{
    GuiPendingLogicalMediaOverride, GuiUserMediaTargetResolutionSource,
};
use sorotte_player_mpv::LifecycleVerificationTrackedLoad;

const LOGICAL_TARGET: &str = "episode.mkv";
const PLEX_STREAM_TARGET: &str = "https://plex.example/video?token=secret";
const FALLBACK_TARGET: &str = "C:/media/fallback.mkv";
const PLAYLIST_GENERATION: u64 = 4;
const PLEX_PLAYLIST_ENTRY_ID: i64 = 91;
const FALLBACK_PLAYLIST_ENTRY_ID: i64 = 92;

fn plex_candidate() -> GuiMediaResolutionCandidate {
    let playlist_uri = sorotte_plex::PlexPlaylistUri {
        machine_identifier: "machine".to_owned(),
        rating_key: "123".to_owned(),
        title: Some("Episode".to_owned()),
        file_name: Some(LOGICAL_TARGET.to_owned()),
        duration_millis: None,
        size_bytes: None,
        media_type: Some(sorotte_plex::PlexMediaType::Episode),
    };
    let stream_target = sorotte_plex::PlexStreamTarget {
        logical_file: LocalFileUpdate::new(LOGICAL_TARGET),
        matched_item: sorotte_plex::PlexMatchedItem {
            rating_key: "123".to_owned(),
            title: "Episode".to_owned(),
            media_type: sorotte_plex::PlexMediaType::Episode,
            duration_millis: None,
        },
        playlist_uri,
        playback_url: sorotte_plex::SecretPlexPlaybackUrl::new(PLEX_STREAM_TARGET),
    };
    let mut plan = GuiMediaResolutionPlan::new(LOGICAL_TARGET);
    plan.push_plex_stream_candidate(stream_target);
    plan.best_candidate().cloned().expect("Plex candidate")
}

fn local_candidate(path: &str) -> GuiMediaResolutionCandidate {
    let mut plan = GuiMediaResolutionPlan::new(LOGICAL_TARGET);
    plan.push_user_media_candidate(
        path.to_owned(),
        GuiUserMediaTargetResolutionSource::QuickLocal,
    );
    plan.best_candidate().cloned().expect("local candidate")
}

fn started(tracked: LifecycleVerificationTrackedLoad) -> StartedMediaLoad {
    StartedMediaLoad {
        feedback_message: "started".to_owned(),
        player_command_id: Some(tracked.command_id),
        player_media_generation: Some(tracked.media_generation),
    }
}

fn seed_plex_playlist_resolution(
    owner: &mut GuiPersistedConfigRuntimeOwner,
    tracked: LifecycleVerificationTrackedLoad,
    row_id: GuiPlaylistEntryId,
    candidate: &GuiMediaResolutionCandidate,
) {
    let logical_file = LocalFileUpdate::new(LOGICAL_TARGET);
    owner.playlist_resolution.generation = PLAYLIST_GENERATION;
    owner.ensure_playlist_resolution_attempt(
        row_id,
        PLAYLIST_GENERATION,
        LOGICAL_TARGET,
        GuiPlaylistSourcePolicy::Automatic,
    );
    owner.begin_playlist_resolution_candidate_load(candidate.clone(), &started(tracked));
    owner.player_local_file = Some(logical_file.clone());
    owner.player_local_file_placeholder = true;
    owner.pending_logical_media_override = Some(GuiPendingLogicalMediaOverride {
        requested_target: LOGICAL_TARGET.to_owned(),
        loaded_target_secret: sorotte_plex::SecretPlexPlaybackUrl::new(PLEX_STREAM_TARGET),
        logical_file,
        user_initiated: false,
        player_command_id: Some(tracked.command_id),
        player_media_generation: Some(tracked.media_generation),
        playlist_row_id: Some(row_id),
        playlist_generation: PLAYLIST_GENERATION,
        load_completed: false,
        logical_file_observed: false,
    });
}

fn bind_plex_attempt(
    harness: &mut MpvLifecycleVerificationHarness,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
    tracked: LifecycleVerificationTrackedLoad,
) {
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            PLEX_PLAYLIST_ENTRY_ID,
            Some(PLEX_STREAM_TARGET.to_owned()),
            false,
        )],
        None,
    );
    let bound = apply_replay_and_acknowledge_batch("resolution C/F bound", harness, client, gui);
    assert!(bound.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptBound {
                attempt_id,
                media_generation,
                command_id: Some(command_id),
                playlist_entry_id: PLEX_PLAYLIST_ENTRY_ID,
            } if attempt_id == tracked.attempt_id
                && media_generation == tracked.media_generation
                && command_id == tracked.command_id
        )
    }));
}

fn drive_timeout(
    harness: &mut MpvLifecycleVerificationHarness,
    client: &mut VerificationClientRuntime,
    gui: &mut GuiPersistedConfigRuntimeOwner,
    tracked: LifecycleVerificationTrackedLoad,
) -> PlayerEventBatch {
    harness.advance_clock(60_000);
    let timeout =
        apply_replay_and_acknowledge_batch("resolution C/F timeout", harness, client, gui);
    assert!(timeout.semantic_outcomes.iter().any(|outcome| {
        matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::Command(command)
                if command.command_id == tracked.command_id
                    && command.media_generation == Some(tracked.media_generation)
                    && command.result == PlayerCommandSemanticResult::CompletionNotObserved
        )
    }));
    assert!(timeout.semantic_outcomes.iter().any(|outcome| {
        matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::LoadAttempt(load)
                if load.attempt_id == tracked.attempt_id
                    && load.media_generation == tracked.media_generation
                    && load.command_id == Some(tracked.command_id)
                    && load.result == PlayerLoadAttemptResult::Indeterminate
        )
    }));
    timeout
}

fn assert_plex_attempt_identity(
    owner: &GuiPersistedConfigRuntimeOwner,
    tracked: LifecycleVerificationTrackedLoad,
    candidate: &GuiMediaResolutionCandidate,
    expected_state: PlaylistResolutionAttemptState,
    fallback_pending: bool,
) {
    let attempt = owner
        .playlist_resolution_attempt
        .as_ref()
        .expect("actual GUI playlist resolution attempt");
    assert_eq!(attempt.target, LOGICAL_TARGET);
    assert_eq!(attempt.candidate.as_ref(), Some(candidate));
    assert_eq!(
        attempt.candidate_provider,
        Some(GuiMediaSourceProviderId::plex_stream())
    );
    assert_eq!(attempt.player_command_id, Some(tracked.command_id));
    assert_eq!(
        attempt.player_media_generation,
        Some(tracked.media_generation)
    );
    assert_eq!(attempt.load_attempt_id, Some(tracked.attempt_id));
    assert_eq!(attempt.state, expected_state);
    assert_eq!(attempt.fallback_pending, fallback_pending);
}

#[test]
fn indeterminate_plex_resolution_recovers_from_late_raw_active_and_matching_file() {
    let mut harness = MpvLifecycleVerificationHarness::new();
    let tracked = harness.accept_tracked_load(PLEX_STREAM_TARGET, []);
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    let row_id = GuiPlaylistEntryId::next();
    let candidate = plex_candidate();
    seed_plex_playlist_resolution(&mut gui, tracked, row_id, &candidate);
    bind_plex_attempt(&mut harness, &mut client, &mut gui, tracked);

    let attempt = gui
        .playlist_resolution_attempt
        .as_ref()
        .expect("bound GUI playlist resolution attempt");
    assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
    assert_eq!(attempt.load_attempt_id, Some(tracked.attempt_id));

    drive_timeout(&mut harness, &mut client, &mut gui, tracked);
    assert_plex_attempt_identity(
        &gui,
        tracked,
        &candidate,
        PlaylistResolutionAttemptState::Indeterminate,
        true,
    );
    assert!(
        gui.playlist_resolution_attempt
            .as_ref()
            .expect("indeterminate GUI playlist resolution attempt")
            .candidate_failures
            .is_empty(),
        "a completion-observation timeout must not permanently exclude the candidate"
    );
    let pending_override = gui
        .pending_logical_media_override
        .as_ref()
        .expect("timeout must retain the correlated logical override");
    assert_eq!(pending_override.requested_target, LOGICAL_TARGET);
    assert_eq!(
        pending_override.loaded_target_secret.as_str(),
        PLEX_STREAM_TARGET
    );
    assert_eq!(pending_override.player_command_id, Some(tracked.command_id));
    assert_eq!(
        pending_override.player_media_generation,
        Some(tracked.media_generation)
    );
    assert_eq!(pending_override.playlist_row_id, Some(row_id));
    assert_eq!(pending_override.playlist_generation, PLAYLIST_GENERATION);
    assert!(!pending_override.load_completed);
    assert!(!pending_override.logical_file_observed);
    assert_eq!(
        gui.player_local_file,
        Some(LocalFileUpdate::new(LOGICAL_TARGET))
    );
    assert!(gui.player_local_file_placeholder);

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLEX_PLAYLIST_ENTRY_ID,
    }));
    let late_start = apply_replay_and_acknowledge_batch(
        "resolution C late start",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(late_start.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptStarting {
                attempt_id,
                media_generation,
                command_id: Some(command_id),
                playlist_entry_id: PLEX_PLAYLIST_ENTRY_ID,
                owns_transport: false,
            } if attempt_id == tracked.attempt_id
                && media_generation == tracked.media_generation
                && command_id == tracked.command_id
        )
    }));

    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": PLEX_STREAM_TARGET,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    let recovered = apply_replay_and_acknowledge_batch(
        "resolution C late active and matching file",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(recovered.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptActive {
                attempt_id,
                media_generation,
                command_id: Some(command_id),
                playlist_entry_id: PLEX_PLAYLIST_ENTRY_ID,
            } if attempt_id == tracked.attempt_id
                && media_generation == tracked.media_generation
                && command_id == tracked.command_id
        )
    }));
    assert!(recovered.events.iter().any(|event| {
        matches!(
            &event.event,
            PlayerEvent::LocalFileChanged {
                attempt_id,
                media_generation,
                update,
            } if *attempt_id == tracked.attempt_id
                && *media_generation == tracked.media_generation
                && update.name == PLEX_STREAM_TARGET
                && update.path.as_deref() == Some(PLEX_STREAM_TARGET)
        )
    }));
    assert!(recovered.semantic_outcomes.iter().all(|outcome| {
        !matches!(
            &outcome.outcome,
            PlayerSemanticOutcome::LoadAttempt(load)
                if load.attempt_id == tracked.attempt_id
                    && load.result == PlayerLoadAttemptResult::Loaded
        )
    }));

    assert_plex_attempt_identity(
        &gui,
        tracked,
        &candidate,
        PlaylistResolutionAttemptState::Active,
        false,
    );
    assert!(
        gui.playlist_resolution_attempt
            .as_ref()
            .expect("recovered GUI playlist resolution attempt")
            .candidate_failures
            .is_empty(),
        "late positive evidence must leave no timeout-only candidate failure"
    );
    assert_eq!(
        gui.player_local_file,
        Some(LocalFileUpdate::new(LOGICAL_TARGET))
    );
    assert!(!gui.player_local_file_placeholder);
    assert!(gui.pending_logical_media_override.is_none());
}

#[test]
fn accepted_fallback_fences_late_raw_activation_of_superseded_plex_attempt() {
    let mut harness = MpvLifecycleVerificationHarness::new();
    let old = harness.accept_tracked_load(PLEX_STREAM_TARGET, []);
    let mut client = verification_client_runtime();
    let mut gui = GuiPersistedConfigRuntimeOwner::default();
    let row_id = GuiPlaylistEntryId::next();
    let old_candidate = plex_candidate();
    seed_plex_playlist_resolution(&mut gui, old, row_id, &old_candidate);
    bind_plex_attempt(&mut harness, &mut client, &mut gui, old);
    drive_timeout(&mut harness, &mut client, &mut gui, old);

    let fallback = harness.accept_tracked_load(FALLBACK_TARGET, [PLEX_PLAYLIST_ENTRY_ID]);
    let fallback_candidate = local_candidate(FALLBACK_TARGET);
    gui.begin_playlist_resolution_candidate_load(fallback_candidate.clone(), &started(fallback));
    let fallback_logical_file =
        LocalFileUpdate::new("fallback.mkv").with_path(FALLBACK_TARGET.to_owned());
    gui.player_local_file = Some(fallback_logical_file.clone());
    gui.player_local_file_placeholder = true;
    gui.pending_logical_media_override = None;

    harness.apply_authoritative_snapshot(
        [
            LifecycleVerificationPlaylistEntry::new(
                PLEX_PLAYLIST_ENTRY_ID,
                Some(PLEX_STREAM_TARGET.to_owned()),
                false,
            ),
            LifecycleVerificationPlaylistEntry::new(
                FALLBACK_PLAYLIST_ENTRY_ID,
                Some(FALLBACK_TARGET.to_owned()),
                false,
            ),
        ],
        None,
    );
    let fallback_bound = apply_replay_and_acknowledge_batch(
        "resolution F fallback accepted and bound",
        &mut harness,
        &mut client,
        &mut gui,
    );
    assert!(fallback_bound.events.iter().any(|event| {
        matches!(
            event.event,
            PlayerEvent::LoadAttemptBound {
                attempt_id,
                media_generation,
                command_id: Some(command_id),
                playlist_entry_id: FALLBACK_PLAYLIST_ENTRY_ID,
            } if attempt_id == fallback.attempt_id
                && media_generation == fallback.media_generation
                && command_id == fallback.command_id
        )
    }));
    assert_eq!(
        harness.projection().attempts[&old.attempt_id].logical_ownership_revoked,
        SnapshotField::Known(true),
        "accepting fallback must logically supersede the indeterminate predecessor"
    );

    {
        let attempt = gui
            .playlist_resolution_attempt
            .as_ref()
            .expect("fallback GUI playlist resolution attempt");
        assert_eq!(attempt.candidate.as_ref(), Some(&fallback_candidate));
        assert_eq!(
            attempt.candidate_provider,
            Some(GuiMediaSourceProviderId::local())
        );
        assert_eq!(attempt.player_command_id, Some(fallback.command_id));
        assert_eq!(
            attempt.player_media_generation,
            Some(fallback.media_generation)
        );
        assert_eq!(attempt.load_attempt_id, Some(fallback.attempt_id));
        assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
        assert!(!attempt.fallback_pending);
    }

    harness.ingest_decoded_mpv_json(json!({
        "event": "start-file",
        "playlist_entry_id": PLEX_PLAYLIST_ENTRY_ID,
    }));
    harness.ingest_decoded_mpv_json(json!({
        "event": "property-change",
        "name": "path",
        "data": PLEX_STREAM_TARGET,
    }));
    harness.ingest_decoded_mpv_json(json!({ "event": "file-loaded" }));
    assert_eq!(
        harness.take_event_batch(),
        None,
        "superseded late evidence must not create an authoritative delivery"
    );
    let producer = harness.projection();
    let client_projection = client.lifecycle_verification_projection();
    let gui_projection = gui_lifecycle_verification_projection(&gui);
    assert_projection_compatible(
        "resolution F suppressed late evidence: adapter to client",
        &producer,
        &client_projection,
    );
    assert_projection_compatible(
        "resolution F suppressed late evidence: adapter to GUI",
        &producer,
        &gui_projection,
    );
    assert_ne!(
        producer.physical_transport_owner,
        SnapshotField::Known(old.attempt_id)
    );
    assert_eq!(
        producer.attempts[&old.attempt_id].semantic_load_result,
        SnapshotField::Known(Some(PlayerLoadAttemptResult::Indeterminate))
    );
    assert_eq!(
        producer.attempts[&old.attempt_id].logical_ownership_revoked,
        SnapshotField::Known(true)
    );

    let attempt = gui
        .playlist_resolution_attempt
        .as_ref()
        .expect("late old evidence must retain fallback GUI attempt");
    assert_eq!(attempt.candidate.as_ref(), Some(&fallback_candidate));
    assert_eq!(
        attempt.candidate_provider,
        Some(GuiMediaSourceProviderId::local())
    );
    assert_eq!(attempt.player_command_id, Some(fallback.command_id));
    assert_eq!(
        attempt.player_media_generation,
        Some(fallback.media_generation)
    );
    assert_eq!(attempt.load_attempt_id, Some(fallback.attempt_id));
    assert_eq!(attempt.state, PlaylistResolutionAttemptState::Loading);
    assert_eq!(gui.player_local_file, Some(fallback_logical_file));
    assert!(gui.player_local_file_placeholder);
    assert!(gui.pending_logical_media_override.is_none());
}
