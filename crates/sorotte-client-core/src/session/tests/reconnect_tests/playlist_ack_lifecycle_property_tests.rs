//! Acknowledgement-fenced reconnect playlist histories.
//!
//! The broader reconnect model in `property_tests` covers retry scheduling,
//! state restoration, and the first server playlist snapshot. This module
//! deliberately starts at the narrower playlist ownership boundary and keeps
//! exploring after restoration has been armed and emitted. Its reference
//! model is phrased in terms of desired playlist ownership rather than the
//! production transition helpers.

use proptest::prelude::*;

use super::*;
use crate::ReconnectPlaylistRestoreIntent;

const RECONNECT_DELAY_SECONDS: f64 = 0.125;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AckPlaylist {
    files: Vec<String>,
    index: Option<i64>,
}

impl AckPlaylist {
    fn empty_with_index(index: Option<i64>) -> Self {
        Self {
            files: Vec::new(),
            index,
        }
    }

    fn restorable(&self) -> Option<Self> {
        if self.files.is_empty() {
            return None;
        }
        let index = self
            .index
            .filter(|index| usize::try_from(*index).is_ok_and(|index| index < self.files.len()));
        Some(Self {
            files: self.files.clone(),
            index,
        })
    }
}

fn ack_playlist_from_intent(intent: &ReconnectPlaylistRestoreIntent) -> AckPlaylist {
    AckPlaylist {
        files: intent.files.clone(),
        index: intent.index,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AckPhase {
    Active { shared_playlists: bool },
    Reconnecting { attempt: u32 },
}

#[derive(Clone, Debug)]
struct AckLifecycleModel {
    phase: AckPhase,
    next_attempt: u32,
    current: Option<AckPlaylist>,
    snapshot: Option<AckPlaylist>,
    intent: Option<AckPlaylist>,
    pending_ack: Option<AckPlaylist>,
}

impl AckLifecycleModel {
    fn new(initial: AckPlaylist) -> Self {
        Self {
            phase: AckPhase::Active {
                shared_playlists: true,
            },
            next_attempt: 0,
            current: Some(initial),
            snapshot: None,
            intent: None,
            pending_ack: None,
        }
    }

    fn begin_reconnect_generation(&mut self) -> (u32, Vec<ClientRuntimeAction>) {
        let attempt = self.next_attempt;
        self.next_attempt = self.next_attempt.saturating_add(1);
        let preserved = self
            .snapshot
            .take()
            .or(self.intent.take())
            .or(self.pending_ack.take());
        let captured_current = self
            .current
            .take()
            .and_then(|playlist| playlist.restorable());
        self.snapshot = preserved.or(captured_current);
        self.intent = None;
        self.pending_ack = None;
        self.phase = AckPhase::Reconnecting { attempt };

        (
            attempt,
            vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::Attempting {
                        retries: attempt,
                        delay_seconds: RECONNECT_DELAY_SECONDS,
                    },
                ),
                ClientRuntimeAction::ScheduleReconnect {
                    delay_seconds: RECONNECT_DELAY_SECONDS,
                },
            ],
        )
    }

    fn apply_hello(&mut self, shared_playlists: bool) {
        assert!(
            matches!(self.phase, AckPhase::Reconnecting { .. }),
            "reference precondition: Hello starts an active reconnect generation"
        );
        self.phase = AckPhase::Active { shared_playlists };
    }

    fn apply_empty_server_update(&mut self) {
        assert!(
            matches!(self.phase, AckPhase::Active { .. }),
            "reference precondition: playlist updates require an active generation"
        );
        if let Some(snapshot) = self.snapshot.take() {
            self.intent = Some(snapshot);
            return;
        }

        let retained_index = self.current.as_ref().and_then(|playlist| playlist.index);
        self.current = Some(AckPlaylist::empty_with_index(retained_index));
    }

    fn apply_authoritative_playlist(&mut self, playlist: AckPlaylist) {
        assert!(
            matches!(self.phase, AckPhase::Active { .. }),
            "reference precondition: playlist updates require an active generation"
        );
        assert!(
            !playlist.files.is_empty(),
            "reference precondition: authoritative replacement is non-empty"
        );
        self.snapshot = None;
        self.intent = None;
        self.pending_ack = None;
        self.current = Some(playlist);
    }

    fn drain_restore(&mut self) -> Vec<ClientRuntimeAction> {
        let AckPhase::Active { shared_playlists } = self.phase else {
            return Vec::new();
        };
        if !shared_playlists {
            self.snapshot = None;
            self.intent = None;
            self.pending_ack = None;
            return Vec::new();
        }
        let Some(restore) = self.intent.take() else {
            return Vec::new();
        };
        self.pending_ack = Some(restore.clone());
        restore_actions(&restore)
    }

    fn apply_matching_echo(&mut self) -> Option<AckPlaylist> {
        let echo = self.pending_ack.clone()?;
        self.apply_authoritative_playlist(echo.clone());
        Some(echo)
    }

    fn assert_matches(&self, session: &ClientSession, context: &str) {
        match (self.phase, session.connection_phase()) {
            (
                AckPhase::Active {
                    shared_playlists: expected,
                },
                ConnectionPhase::Active(actual),
            ) => assert_eq!(
                actual.shared_playlists, expected,
                "{context}: shared-playlist capability drift"
            ),
            (
                AckPhase::Reconnecting { attempt: expected },
                ConnectionPhase::Reconnecting { attempt: actual },
            ) => assert_eq!(*actual, expected, "{context}: reconnect attempt drift"),
            (expected, actual) => {
                panic!("{context}: reconnect phase drift: expected {expected:?}, actual {actual:?}")
            }
        }
        assert_eq!(
            session.model.reconnect.in_progress,
            matches!(self.phase, AckPhase::Reconnecting { .. }),
            "{context}: reconnect-in-progress drift"
        );
        assert_eq!(
            session
                .model
                .reconnect
                .playlist_restore_snapshot
                .as_ref()
                .map(ack_playlist_from_intent),
            self.snapshot,
            "{context}: captured restore snapshot drift"
        );
        assert_eq!(
            session
                .model
                .reconnect
                .playlist_restore_intent
                .as_ref()
                .map(ack_playlist_from_intent),
            self.intent,
            "{context}: armed restore intent drift"
        );
        assert_eq!(
            session
                .model
                .reconnect
                .playlist_restore_pending_ack
                .as_ref()
                .map(ack_playlist_from_intent),
            self.pending_ack,
            "{context}: emitted restore acknowledgement fence drift"
        );
        match (&self.current, session.current_room_playlist()) {
            (None, None) => {}
            (Some(expected), Some(actual)) => {
                assert_eq!(
                    actual.files, expected.files,
                    "{context}: current playlist files drift"
                );
                assert_eq!(
                    actual.index, expected.index,
                    "{context}: current playlist index drift"
                );
            }
            (expected, actual) => panic!(
                "{context}: current playlist presence drift: expected {expected:?}, actual {actual:?}"
            ),
        }
    }
}

fn restore_actions(playlist: &AckPlaylist) -> Vec<ClientRuntimeAction> {
    let mut actions = vec![
        ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::RestoringPlaylist,
        ),
        ClientRuntimeAction::SetPlaylist {
            files: playlist.files.clone(),
        },
    ];
    if let Some(index) = playlist.index {
        actions.push(ClientRuntimeAction::SetPlaylistIndex { index });
    }
    actions
}

#[derive(Clone, Debug)]
enum GenerationSnapshot {
    Deferred,
    Empty,
    Authoritative {
        variant: u8,
        file_count: u8,
        index_seed: u8,
    },
}

#[derive(Clone, Debug)]
enum AckLifecycleStep {
    Reconnect {
        shared_playlists: bool,
        snapshot: GenerationSnapshot,
    },
    Drain,
    MatchingEcho,
    EmptyServerUpdate,
    DivergentAuthority {
        variant: u8,
        file_count: u8,
        index_seed: u8,
    },
}

fn initial_playlist_strategy() -> impl Strategy<Value = AckPlaylist> {
    (0_u8..=31, 1_u8..=4, any::<bool>()).prop_map(|(variant, file_count, include_index)| {
        let files = (0..file_count)
            .map(|position| format!("local-{variant}-{position}.mkv"))
            .collect::<Vec<_>>();
        let index = include_index.then_some(i64::from(variant % file_count));
        AckPlaylist { files, index }
    })
}

fn generation_snapshot_strategy() -> impl Strategy<Value = GenerationSnapshot> {
    prop_oneof![
        1 => Just(GenerationSnapshot::Deferred),
        3 => Just(GenerationSnapshot::Empty),
        2 => (0_u8..=63, 1_u8..=4, any::<u8>()).prop_map(
            |(variant, file_count, index_seed)| GenerationSnapshot::Authoritative {
                variant,
                file_count,
                index_seed,
            }
        ),
    ]
}

fn ack_lifecycle_step_strategy() -> impl Strategy<Value = AckLifecycleStep> {
    prop_oneof![
        5 => (any::<bool>(), generation_snapshot_strategy()).prop_map(
            |(shared_playlists, snapshot)| AckLifecycleStep::Reconnect {
                shared_playlists,
                snapshot,
            }
        ),
        4 => Just(AckLifecycleStep::Drain),
        3 => Just(AckLifecycleStep::MatchingEcho),
        2 => Just(AckLifecycleStep::EmptyServerUpdate),
        4 => (0_u8..=63, 1_u8..=4, any::<u8>()).prop_map(
            |(variant, file_count, index_seed)| AckLifecycleStep::DivergentAuthority {
                variant,
                file_count,
                index_seed,
            }
        ),
    ]
}

fn authoritative_playlist(variant: u8, file_count: u8, index_seed: u8) -> AckPlaylist {
    let files = (0..file_count)
        .map(|position| format!("authority-{variant}-{position}.mkv"))
        .collect::<Vec<_>>();
    AckPlaylist {
        files,
        index: Some(i64::from(index_seed % file_count)),
    }
}

fn apply_json(session: &mut ClientSession, message: Value) {
    session
        .apply_message_json(&message.to_string())
        .expect("acknowledgement-lifecycle protocol message should apply");
}

fn apply_hello(session: &mut ClientSession, shared_playlists: bool) {
    apply_json(
        session,
        json!({
            "Hello": {
                "username": "alice",
                "room": {"name": "room1"},
                "version": "1.7.5",
                "features": {"sharedPlaylists": shared_playlists},
            }
        }),
    );
}

fn apply_empty_server_update(session: &mut ClientSession) {
    apply_json(session, json!({"Set": {"playlistChange": {"files": []}}}));
}

fn apply_playlist_update(session: &mut ClientSession, playlist: &AckPlaylist, user: Option<&str>) {
    let change = match user {
        Some(user) => json!({
            "Set": {
                "playlistChange": {
                    "files": playlist.files,
                    "user": user,
                }
            }
        }),
        None => json!({"Set": {"playlistChange": {"files": playlist.files}}}),
    };
    apply_json(session, change);
    if let Some(index) = playlist.index {
        let index_change = match user {
            Some(user) => json!({
                "Set": {
                    "playlistIndex": {
                        "index": index,
                        "user": user,
                    }
                }
            }),
            None => json!({"Set": {"playlistIndex": {"index": index}}}),
        };
        apply_json(session, index_change);
    }
}

fn seeded_session(initial: &AckPlaylist) -> ClientSession {
    let mut session = ClientSession::default();
    session.reconnect_policy_mut().max_retries = u32::MAX;
    session.reconnect_policy_mut().base_delay_seconds = RECONNECT_DELAY_SECONDS;
    session.reconnect_policy_mut().max_backoff_exponent = 0;
    apply_hello(&mut session, true);
    apply_playlist_update(&mut session, initial, Some("alice"));
    session
}

fn begin_generation(
    session: &mut ClientSession,
    model: &mut AckLifecycleModel,
    shared_playlists: bool,
    snapshot: &GenerationSnapshot,
    context: &str,
) {
    let (attempt, expected_actions) = model.begin_reconnect_generation();
    let actual_actions = session.runtime_actions_for_reconnect_retry(attempt);
    assert_eq!(
        actual_actions, expected_actions,
        "{context}: reconnect scheduling actions drift"
    );
    model.assert_matches(
        session,
        &format!("{context}: disconnected generation {attempt}"),
    );

    model.apply_hello(shared_playlists);
    apply_hello(session, shared_playlists);
    model.assert_matches(session, &format!("{context}: Hello generation {attempt}"));

    match snapshot {
        GenerationSnapshot::Deferred => {}
        GenerationSnapshot::Empty => {
            model.apply_empty_server_update();
            apply_empty_server_update(session);
        }
        GenerationSnapshot::Authoritative {
            variant,
            file_count,
            index_seed,
        } => {
            let playlist = authoritative_playlist(*variant, *file_count, *index_seed);
            model.apply_authoritative_playlist(playlist.clone());
            apply_playlist_update(session, &playlist, None);
        }
    }
    model.assert_matches(
        session,
        &format!("{context}: initial server snapshot generation {attempt}"),
    );
}

fn apply_step(
    session: &mut ClientSession,
    model: &mut AckLifecycleModel,
    step: &AckLifecycleStep,
    context: &str,
) {
    match step {
        AckLifecycleStep::Reconnect {
            shared_playlists,
            snapshot,
        } => begin_generation(session, model, *shared_playlists, snapshot, context),
        AckLifecycleStep::Drain => {
            let expected = model.drain_restore();
            let actual = session.runtime_actions_for_reconnect_playlist_restore_if_needed();
            assert_eq!(actual, expected, "{context}: playlist restore drain drift");
            model.assert_matches(session, context);
        }
        AckLifecycleStep::MatchingEcho => {
            if let Some(echo) = model.apply_matching_echo() {
                apply_playlist_update(session, &echo, Some("alice"));
            }
            model.assert_matches(session, context);
        }
        AckLifecycleStep::EmptyServerUpdate => {
            model.apply_empty_server_update();
            apply_empty_server_update(session);
            model.assert_matches(session, context);
        }
        AckLifecycleStep::DivergentAuthority {
            variant,
            file_count,
            index_seed,
        } => {
            let playlist = authoritative_playlist(*variant, *file_count, *index_seed);
            model.apply_authoritative_playlist(playlist.clone());
            apply_playlist_update(session, &playlist, None);
            model.assert_matches(session, context);
        }
    }
}

fn prime_pending_ack(
    session: &mut ClientSession,
    model: &mut AckLifecycleModel,
    context: &str,
) -> Vec<ClientRuntimeAction> {
    begin_generation(session, model, true, &GenerationSnapshot::Empty, context);
    assert!(
        model.intent.is_some() && model.snapshot.is_none() && model.pending_ack.is_none(),
        "{context}: empty initial snapshot must arm exactly one restore intent"
    );
    let expected = model.drain_restore();
    let actual = session.runtime_actions_for_reconnect_playlist_restore_if_needed();
    assert_eq!(
        actual, expected,
        "{context}: initial restore emission drift"
    );
    assert!(
        model.pending_ack.is_some() && model.snapshot.is_none() && model.intent.is_none(),
        "{context}: emitted restore must be retained behind the acknowledgement fence"
    );
    model.assert_matches(session, &format!("{context}: awaiting acknowledgement"));
    actual
}

proptest! {
    #![proptest_config(super::property_tests::configured_proptest())]

    #[test]
    fn generated_acknowledgement_fenced_playlist_histories_match_reference_model(
        initial in initial_playlist_strategy(),
        steps in prop::collection::vec(ack_lifecycle_step_strategy(), 1..=64),
    ) {
        let mut session = seeded_session(&initial);
        let mut model = AckLifecycleModel::new(initial);
        model.assert_matches(&session, "generated initial state");
        let _ = prime_pending_ack(&mut session, &mut model, "generated mandatory lifecycle");

        for (index, step) in steps.iter().enumerate() {
            apply_step(
                &mut session,
                &mut model,
                step,
                &format!("generated acknowledgement step {index}: {step:?}"),
            );
        }
    }
}

fn deterministic_initial_playlist() -> AckPlaylist {
    AckPlaylist {
        files: vec!["episode-1.mkv".to_owned(), "episode-2.mkv".to_owned()],
        index: Some(1),
    }
}

#[test]
fn pending_restore_survives_multiple_reconnect_generations_until_matching_echo() {
    let initial = deterministic_initial_playlist();
    let mut session = seeded_session(&initial);
    let mut model = AckLifecycleModel::new(initial.clone());
    let first_emission = prime_pending_ack(&mut session, &mut model, "first reconnect generation");
    assert_eq!(first_emission, restore_actions(&initial));

    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "repeated drain while awaiting acknowledgement",
    );
    begin_generation(
        &mut session,
        &mut model,
        true,
        &GenerationSnapshot::Empty,
        "second reconnect before acknowledgement",
    );
    assert_eq!(
        model.intent,
        Some(initial.clone()),
        "the next generation must re-arm the unacknowledged desired playlist"
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "second generation restore emission",
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::MatchingEcho,
        "matching server echo",
    );

    assert_eq!(model.current, Some(initial));
    assert!(model.snapshot.is_none());
    assert!(model.intent.is_none());
    assert!(model.pending_ack.is_none());
}

#[test]
fn divergent_authority_supersedes_both_armed_and_emitted_restore_ownership() {
    let initial = deterministic_initial_playlist();
    let mut session = seeded_session(&initial);
    let mut model = AckLifecycleModel::new(initial);
    begin_generation(
        &mut session,
        &mut model,
        true,
        &GenerationSnapshot::Empty,
        "armed restore generation",
    );
    assert!(model.intent.is_some(), "precondition: restore is armed");

    let first_authority = AckLifecycleStep::DivergentAuthority {
        variant: 41,
        file_count: 2,
        index_seed: 0,
    };
    apply_step(
        &mut session,
        &mut model,
        &first_authority,
        "authority supersedes armed restore",
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "superseded armed restore cannot emit",
    );

    begin_generation(
        &mut session,
        &mut model,
        true,
        &GenerationSnapshot::Empty,
        "authoritative playlist becomes next desired snapshot",
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "authoritative playlist restore emission",
    );
    assert!(
        model.pending_ack.is_some(),
        "precondition: replacement restore is awaiting acknowledgement"
    );

    let second_authority = authoritative_playlist(42, 3, 2);
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::DivergentAuthority {
            variant: 42,
            file_count: 3,
            index_seed: 2,
        },
        "authority supersedes emitted restore",
    );
    assert_eq!(model.current, Some(second_authority));
    assert!(model.snapshot.is_none());
    assert!(model.intent.is_none());
    assert!(model.pending_ack.is_none());
}

#[test]
fn shared_playlist_disablement_discards_restore_across_later_generations() {
    let initial = deterministic_initial_playlist();
    let mut session = seeded_session(&initial);
    let mut model = AckLifecycleModel::new(initial);
    let _ = prime_pending_ack(&mut session, &mut model, "shared generation");

    begin_generation(
        &mut session,
        &mut model,
        false,
        &GenerationSnapshot::Empty,
        "shared-playlist-disabled generation",
    );
    assert!(
        model.intent.is_some(),
        "the snapshot is still mechanically armed until capability policy is drained"
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "disabled capability discards restore ownership",
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "disabled capability repeated drain",
    );

    begin_generation(
        &mut session,
        &mut model,
        true,
        &GenerationSnapshot::Empty,
        "later shared-playlist generation",
    );
    apply_step(
        &mut session,
        &mut model,
        &AckLifecycleStep::Drain,
        "discarded restore cannot reappear",
    );
    assert!(model.snapshot.is_none());
    assert!(model.intent.is_none());
    assert!(model.pending_ack.is_none());
}
