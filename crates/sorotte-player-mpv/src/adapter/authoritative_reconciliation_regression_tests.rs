use super::*;
use crate::lifecycle::LoadLifecycleReconciliation;
use std::{collections::VecDeque, io};

#[test]
fn mismatched_authoritative_current_terminalizes_predecessor_before_external_admission() {
    let mut adapter = MpvAdapter::default();
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch,
        media_generation: PlayerMediaGeneration::new(1),
        playlist_entry_id: 100,
        observed_target: "C:/media/original.mkv".to_owned(),
        file_loaded: false,
    });
    let predecessor = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("initial external predecessor");
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptSubmitted {
        command_id: Some(PlayerCommandId::new(1)),
        media_generation: PlayerMediaGeneration::new(2),
        requested_target: "C:/media/commanded.mkv".to_owned(),
        baseline_playlist_entry_ids: BTreeSet::from([100]),
    });
    let pending = adapter
        .player_lifecycle
        .attempt_for_command(PlayerCommandId::new(1))
        .expect("pending commanded successor");
    let entries = vec![AuthoritativePlaylistEntry::new(
        101,
        Some("C:/media/external.mkv".to_owned()),
        true,
    )];
    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch,
        entries: entries.clone(),
        current_path: Some("C:/media/external.mkv".to_owned()),
    });

    adapter.observe_external_current_from_authority(&entries, Some("C:/media/external.mkv"));

    adapter
        .player_lifecycle
        .assert_invariants()
        .expect("authoritative external ingress must preserve lifecycle invariants");
    let selected = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("authoritative external successor");
    assert_ne!(selected, pending);
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&selected].playlist_entry_id,
        Some(101)
    );
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&predecessor].superseded_by, None,
        "the authoritative snapshot terminalizes a contradicted predecessor before \
         admitting the external current entry"
    );
    assert!(
        adapter.player_lifecycle.load_attempts[&predecessor]
            .state
            .is_terminal()
    );
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&selected].replaced_attempt, None,
        "an external current entry admitted after terminalization has no live predecessor"
    );
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&pending].replaced_attempt,
        Some(predecessor),
        "the unselected pending attempt may retain historical provenance once the \
         predecessor is terminal and has no selected successor"
    );
}

#[test]
fn accepted_load_detaches_a_rejected_successor_claim() {
    let mut adapter = MpvAdapter::default();
    let attachment_epoch = adapter.lifecycle_epoch();
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch,
        media_generation: PlayerMediaGeneration::new(1),
        playlist_entry_id: 100,
        observed_target: "C:/media/original.mkv".to_owned(),
        file_loaded: true,
    });
    let predecessor = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("initial external predecessor");
    let rejected = adapter.submit_lifecycle_load(
        Some(PlayerCommandId::new(1)),
        PlayerMediaGeneration::new(2),
        "C:/media/rejected.mkv",
        BTreeSet::from([100]),
    );
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptRejected {
        attachment_epoch,
        attempt_id: rejected,
        failure: PlayerCommandFailureKind::Unknown,
    });
    let selected = adapter.submit_lifecycle_load(
        Some(PlayerCommandId::new(2)),
        PlayerMediaGeneration::new(3),
        "C:/media/selected.mkv",
        BTreeSet::from([100]),
    );

    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch,
        attempt_id: selected,
    });

    adapter
        .player_lifecycle
        .assert_invariants()
        .expect("adapter acknowledgement ingress must preserve lifecycle invariants");
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&predecessor].superseded_by,
        Some(selected)
    );
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&selected].replaced_attempt,
        Some(predecessor)
    );
    assert_eq!(
        adapter.player_lifecycle.load_attempts[&rejected].replaced_attempt, None,
        "a rejected, unselected load must not keep a backlink to the selected predecessor"
    );
}

#[derive(Debug, Default)]
struct InterleavedAuthorityTransport {
    pending_lines: VecDeque<String>,
    pause_response: bool,
    paused_for_cache_response: bool,
    seeking_response: bool,
    core_idle_response: bool,
    pause_event_after_response: Option<bool>,
    verified_transition_before_playlist_response: bool,
}

impl InterleavedAuthorityTransport {
    fn response_data(&self, property: &str) -> Value {
        let current_path = if self.verified_transition_before_playlist_response {
            "https://media.example.test/cap.wav"
        } else {
            "C:/media/current.mkv"
        };
        match property {
            MPV_PROPERTY_PLAYLIST => json!([{
                "id": 41,
                "filename": current_path,
                "current": true,
                "playing": true,
            }]),
            MPV_PROPERTY_PATH => json!(current_path),
            MPV_PROPERTY_PAUSE => json!(self.pause_response),
            MPV_PROPERTY_TIME_POS => json!(12.0),
            MPV_PROPERTY_SPEED => json!(1.0),
            MPV_PROPERTY_PAUSED_FOR_CACHE => json!(self.paused_for_cache_response),
            MPV_PROPERTY_CACHE_BUFFERING_STATE => json!(100.0),
            MPV_PROPERTY_SEEKING => json!(self.seeking_response),
            MPV_PROPERTY_SEEKABLE => json!(true),
            MPV_PROPERTY_CORE_IDLE => json!(self.core_idle_response),
            MPV_PROPERTY_DEMUXER_CACHE_IDLE => json!(false),
            MPV_PROPERTY_EOF_REACHED => json!(false),
            _ => Value::Null,
        }
    }
}

impl MpvJsonIpcTransport for InterleavedAuthorityTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim()).expect("valid IPC request");
        let request_id = request["request_id"].as_u64().expect("request id");
        let property = request["command"][1].as_str().expect("get-property name");
        if property == MPV_PROPERTY_PLAYLIST && self.verified_transition_before_playlist_response {
            self.pending_lines.push_back(format!(
                "{}\n",
                json!({
                    "event": MPV_EVENT_START_FILE,
                    "playlist_entry_id": 41,
                })
            ));
            let payload = json!({
                "protocol": SOROTTE_NETWORK_OPTIONS_PROTOCOL,
                "ownerId": "causal-test-owner",
                "attachmentId": "causal-test-attachment",
                "configurationGeneration": 2,
                "hookInstanceId": "test-hook-instance",
                "loadSequence": 1,
                "sourcePath": "https://media.example.test/cap.wav",
                "streamOpenFilename": "https://media.example.test/cap.wav",
                "status": "network-updated",
                "applicationState": "applied",
                "verification": "complete",
                "optionResults": [{
                    "name": "cache-secs",
                    "status": "applied",
                }],
                "effectiveOptions": {
                    "cache-secs": "60",
                },
            });
            self.pending_lines.push_back(format!(
                "{}\n",
                json!({
                    "event": "client-message",
                    "args": [
                        SOROTTE_NETWORK_OPTIONS_CLIENT_MESSAGE_TRANSITION_RESULT,
                        payload.to_string(),
                    ],
                })
            ));
        }
        self.pending_lines.push_back(format!(
            "{}\n",
            json!({
                "request_id": request_id,
                "error": "success",
                "data": self.response_data(property),
            })
        ));
        if property == MPV_PROPERTY_PAUSE
            && let Some(paused) = self.pause_event_after_response
        {
            // Emitted after the pause response; the worker consumes this
            // while waiting for the following time-pos response.
            self.pending_lines.push_back(format!(
                "{}\n",
                json!({
                    "event": MPV_EVENT_PROPERTY_CHANGE,
                    "name": MPV_PROPERTY_PAUSE,
                    "data": paused,
                })
            ));
        }
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        let next = self
            .pending_lines
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no scripted response"))?;
        line.clear();
        line.push_str(&next);
        Ok(line.len())
    }
}

#[derive(Debug, Default)]
struct PlaylistThenDisconnectTransport {
    pending_lines: VecDeque<String>,
}

impl MpvJsonIpcTransport for PlaylistThenDisconnectTransport {
    fn send_line_until(&mut self, line: &str, _deadline: Instant) -> io::Result<()> {
        let request: Value = serde_json::from_str(line.trim()).expect("valid IPC request");
        let request_id = request["request_id"].as_u64().expect("request id");
        let property = request["command"][1].as_str().expect("get-property name");
        if property == MPV_PROPERTY_PLAYLIST {
            self.pending_lines.push_back(format!(
                "{}\n",
                json!({
                    "request_id": request_id,
                    "error": "success",
                    "data": [{
                        "id": 41,
                        "filename": "C:/media/current.mkv",
                        "current": true,
                        "playing": true,
                    }],
                })
            ));
        }
        Ok(())
    }

    fn read_line_until(&mut self, line: &mut String, _deadline: Instant) -> io::Result<usize> {
        if let Some(next) = self.pending_lines.pop_front() {
            line.clear();
            line.push_str(&next);
            return Ok(line.len());
        }
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "scripted disconnect after playlist response",
        ))
    }
}

#[test]
fn authoritative_reconciliation_does_not_overwrite_newer_buffered_pause_event() {
    let mut adapter = MpvAdapter::with_test_transport(InterleavedAuthorityTransport {
        pause_event_after_response: Some(true),
        ..InterleavedAuthorityTransport::default()
    });

    adapter.reconcile_lifecycle_from_authority();

    assert_eq!(
        adapter.observed_state.paused,
        Some(true),
        "the pause event emitted after the pause response must remain authoritative"
    );
}

#[test]
fn authoritative_playlist_binding_preserves_earlier_verified_hook_result() {
    let target = "https://media.example.test/cap.wav";
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter =
        MpvAdapter::with_network_options_hook_test_transport(InterleavedAuthorityTransport {
            verified_transition_before_playlist_response: true,
            ..InterleavedAuthorityTransport::default()
        });
    adapter.legacy_syncplayintf_owner_id = "causal-test-owner".to_owned();
    adapter.legacy_syncplayintf_attachment_id = "causal-test-attachment".to_owned();
    adapter.configure_network_media_options([("cache-secs", "60")]);
    adapter.prepare_test_network_options_hook_v3_reducer();
    let attempt_id = adapter.submit_lifecycle_load(None, generation, target, BTreeSet::new());
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch: adapter.lifecycle_epoch(),
        attempt_id,
    });
    adapter.pending_load_request = Some(target.to_owned());
    adapter.pending_load_generation = Some(generation);

    adapter.reconcile_lifecycle_from_authority();

    let diagnostics = adapter.network_media_diagnostic_snapshot();
    assert_eq!(diagnostics.media_generation, Some(generation));
    assert_eq!(diagnostics.load_sequence, Some(1));
    assert_eq!(
        diagnostics.application_state,
        Some(MpvNetworkMediaPolicyApplicationState::Applied)
    );
    assert!(
        diagnostics.verification_complete,
        "binding start-file after the playlist response must not erase an earlier causal hook result"
    );
    assert_eq!(
        diagnostics.effective_cache_options,
        BTreeMap::from([("cache-secs".to_owned(), "60".to_owned())])
    );
}

#[test]
fn authoritative_reconciliation_preserves_explicit_pause_during_cache_stall() {
    let mut adapter = MpvAdapter::with_test_transport(InterleavedAuthorityTransport {
        pause_response: true,
        paused_for_cache_response: true,
        ..InterleavedAuthorityTransport::default()
    });
    adapter.logical_pause_explicit = true;

    adapter.reconcile_lifecycle_from_authority();

    assert_eq!(adapter.observed_state.paused, Some(true));
    assert_eq!(adapter.observed_state.paused_for_cache, Some(true));
    assert_eq!(
        adapter.observed_state.logical_pause,
        Some(true),
        "an authoritative refresh must not reclassify an explicitly owned pause as cache-only"
    );
}

#[test]
fn authoritative_unpause_clears_explicit_pause_ownership() {
    let mut adapter = MpvAdapter::with_test_transport(InterleavedAuthorityTransport::default());
    adapter.logical_pause_explicit = true;

    adapter.reconcile_lifecycle_from_authority();

    assert_eq!(adapter.observed_state.paused, Some(false));
    assert!(
        !adapter.logical_pause_explicit,
        "an authoritative unpause must retire stale explicit-pause ownership before a later cache-only pause"
    );
}

#[test]
fn authoritative_reconciliation_normalizes_paused_internal_seek_like_event_ingress() {
    let target = "C:/media/paused-internal-seek.wav";
    let generation = PlayerMediaGeneration::new(41);
    let mut adapter = MpvAdapter::with_test_transport(InterleavedAuthorityTransport {
        pause_response: true,
        paused_for_cache_response: false,
        seeking_response: true,
        core_idle_response: true,
        ..InterleavedAuthorityTransport::default()
    });
    adapter.apply_lifecycle_input(PlayerLifecycleInput::ExternalLoadObserved {
        attachment_epoch: adapter.lifecycle_epoch(),
        media_generation: generation,
        playlist_entry_id: 41,
        observed_target: target.to_owned(),
        file_loaded: true,
    });
    let attempt_id = adapter
        .player_lifecycle
        .active_load_attempt
        .expect("external fixture should establish an active attempt");
    adapter.install_physical_projection(
        attempt_id,
        generation,
        Some(41),
        Some(target.to_owned()),
        true,
    );
    adapter.logical_pause_explicit = true;
    adapter.active_generation_has_restarted = true;
    adapter.playback_restart_sequence = 1;
    adapter.transport_phase = PlayerTransportPhase::ReadyPaused;

    adapter.reconcile_lifecycle_from_authority();

    assert_eq!(
        adapter.observed_state.seeking,
        Some(false),
        "authoritative polling must apply the same paused internal-resync normalization as property-event ingress"
    );
    assert_eq!(
        adapter.transport_phase,
        PlayerTransportPhase::ReadyPaused,
        "a reconciliation poll must not re-latch settled paused playback in Seeking"
    );
}

#[test]
fn polled_load_completion_finishes_the_corresponding_tracked_load() {
    let target = "C:/media/polled-before-file-loaded.wav";
    let generation = PlayerMediaGeneration::new(1);
    let mut adapter = MpvAdapter::simulated();
    let command_id = adapter.register_tracked_command(
        Some(generation),
        TrackedCommandKind::Load {
            file_loaded: false,
            ready: false,
        },
    );
    let attempt_id =
        adapter.submit_lifecycle_load(Some(command_id), generation, target, BTreeSet::new());
    adapter.apply_lifecycle_input(PlayerLifecycleInput::LoadAttemptAccepted {
        attachment_epoch: adapter.lifecycle_epoch(),
        attempt_id,
    });
    adapter.accept_tracked_command(command_id);
    adapter.apply_lifecycle_input(PlayerLifecycleInput::PlaylistSnapshot {
        attachment_epoch: adapter.lifecycle_epoch(),
        entries: vec![AuthoritativePlaylistEntry::new(
            41,
            Some(target.to_owned()),
            true,
        )],
        current_path: Some(target.to_owned()),
    });
    adapter.pending_load_request = Some(target.to_owned());
    adapter.pending_load_generation = Some(generation);
    adapter.observed_state.paused = Some(true);
    adapter.observed_state.logical_pause = Some(true);
    adapter.observed_state.paused_for_cache = Some(false);
    adapter.logical_pause_explicit = true;

    assert!(
        adapter.complete_pending_load_request_from_polled_update_if_ready(
            MpvAdapter::local_file_update_for_path(target)
                .with_duration_seconds(8.0)
                .with_size_bytes(768_044),
        ),
        "coherent local-file metadata should complete the pending load"
    );

    assert!(
        adapter
            .pending_tracked_commands
            .iter()
            .all(|command| command.id != command_id),
        "the same polled boundary that completes lifecycle ownership must also finish the tracked load"
    );
    assert!(
        adapter
            .pending_command_progress_updates
            .iter()
            .any(|progress| progress.command_id == command_id && progress.is_terminal()),
        "tracked completion should remain available to legacy progress consumers"
    );
}

#[test]
fn fatal_post_playlist_read_does_not_resolve_partial_authority_snapshot() {
    let mut adapter = MpvAdapter::with_test_transport(PlaylistThenDisconnectTransport::default());

    adapter.reconcile_lifecycle_from_authority();
    assert!(
        adapter
            .ipc_client
            .as_ref()
            .is_some_and(|client| !client.is_healthy()),
        "the scripted path read must fatally disconnect the IPC client"
    );
    // Exercise the normal adjacent pump as well: it currently does not
    // convert the unhealthy client into lifecycle failure/terminal state.
    adapter.maintain_runtime_integrations();

    assert!(
        adapter.player_lifecycle.active_load_attempt.is_none(),
        "a playlist-only partial read must not manufacture active ownership"
    );
    assert_eq!(
        adapter.player_lifecycle.last_reconciliation,
        Some(LoadLifecycleReconciliation::TransportFailure),
        "a fatal path read must not be erased as an unavailable property"
    );
    assert!(
        adapter.player_lifecycle.reconciliation_required,
        "fatal authority acquisition must remain scheduled for reconciliation"
    );
}
