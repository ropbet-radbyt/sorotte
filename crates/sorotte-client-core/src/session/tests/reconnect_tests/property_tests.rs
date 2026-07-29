//! Shrinkable reconnect restore schedules with an independent reference model.
//!
//! The example reconnect tests remain the readable behavior specifications.
//! This module explores valid event schedules around the boundary between a
//! captured pre-disconnect snapshot, the Hello that arms restoration, server
//! playlist authority, and one-shot runtime drains. The model intentionally
//! contains only the externally meaningful reconnect state; it does not copy
//! the production model or call production transition helpers.

use std::collections::BTreeSet;

use proptest::{prelude::*, test_runner::Config as ProptestConfig};

use super::*;

const REFERENCE_MAX_RETRIES: u32 = 3;
const REFERENCE_BASE_DELAY_SECONDS: f64 = 0.25;
const REFERENCE_MAX_BACKOFF_EXPONENT: u32 = 2;
const DEFAULT_PROPTEST_CASES: u32 = 128;
const MAX_PROPTEST_CASES: u32 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileSpec {
    name: String,
    size: u64,
    duration_seconds: u16,
}

impl FileSpec {
    fn payload(&self) -> FilePayload {
        protocol_file_payload(json!({
            "name": self.name,
            "size": self.size,
            "duration": f64::from(self.duration_seconds),
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlaylistSpec {
    files: Vec<String>,
    index: Option<i64>,
}

impl PlaylistSpec {
    fn restore_intent(&self) -> Option<Self> {
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

#[derive(Clone, Debug)]
struct RestoreSeed {
    ready: bool,
    file: Option<FileSpec>,
    playlist: PlaylistSpec,
    fallback_shared_playlists: bool,
    fallback_empty_server_playlist: bool,
    fallback_server_playlist: PlaylistSpec,
}

#[derive(Clone, Debug)]
enum ReconnectStep {
    Retry(u8),
    Hello { shared_playlists: bool },
    EmptyServerPlaylist,
    NonEmptyServerPlaylist { variant: u8, index: Option<i64> },
    DrainTransition,
    DrainState,
    DrainPlaylist,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ReconnectStepKind {
    Retry,
    Hello,
    EmptyServerPlaylist,
    NonEmptyServerPlaylist,
    DrainTransition,
    DrainState,
    DrainPlaylist,
}

const ALL_RECONNECT_STEP_KINDS: &[ReconnectStepKind] = &[
    ReconnectStepKind::Retry,
    ReconnectStepKind::Hello,
    ReconnectStepKind::EmptyServerPlaylist,
    ReconnectStepKind::NonEmptyServerPlaylist,
    ReconnectStepKind::DrainTransition,
    ReconnectStepKind::DrainState,
    ReconnectStepKind::DrainPlaylist,
];

impl ReconnectStep {
    fn kind(&self) -> ReconnectStepKind {
        match self {
            Self::Retry(_) => ReconnectStepKind::Retry,
            Self::Hello { .. } => ReconnectStepKind::Hello,
            Self::EmptyServerPlaylist => ReconnectStepKind::EmptyServerPlaylist,
            Self::NonEmptyServerPlaylist { .. } => ReconnectStepKind::NonEmptyServerPlaylist,
            Self::DrainTransition => ReconnectStepKind::DrainTransition,
            Self::DrainState => ReconnectStepKind::DrainState,
            Self::DrainPlaylist => ReconnectStepKind::DrainPlaylist,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReferencePhase {
    Active { shared_playlists: bool },
    Reconnecting { attempt: u32 },
    Disconnected,
}

#[derive(Clone, Debug)]
struct ReconnectReferenceModel {
    phase: ReferencePhase,
    reconnect_in_progress: bool,
    connected_intent: bool,
    awaiting_server_playlist: bool,
    current_ready: Option<bool>,
    current_file: Option<FileSpec>,
    current_playlist: Option<PlaylistSpec>,
    ready_snapshot: Option<bool>,
    ready_intent: Option<bool>,
    file_snapshot: Option<FileSpec>,
    file_intent: Option<FileSpec>,
    playlist_snapshot: Option<PlaylistSpec>,
    playlist_intent: Option<PlaylistSpec>,
}

impl ReconnectReferenceModel {
    fn from_seed(seed: &RestoreSeed) -> Self {
        Self {
            phase: ReferencePhase::Active {
                shared_playlists: true,
            },
            reconnect_in_progress: false,
            connected_intent: false,
            awaiting_server_playlist: false,
            current_ready: Some(seed.ready),
            current_file: seed.file.clone(),
            current_playlist: Some(seed.playlist.clone()),
            ready_snapshot: None,
            ready_intent: None,
            file_snapshot: None,
            file_intent: None,
            playlist_snapshot: None,
            playlist_intent: None,
        }
    }

    fn reset_for_retry(&mut self, retries: u32) -> Vec<ClientRuntimeAction> {
        self.ready_snapshot = self
            .ready_snapshot
            .take()
            .or(self.ready_intent.take())
            .or(self.current_ready);
        self.file_snapshot = self
            .file_snapshot
            .take()
            .or(self.file_intent.take())
            .or(self.current_file.clone());
        self.playlist_snapshot = self
            .playlist_snapshot
            .take()
            .or(self.playlist_intent.take())
            .or_else(|| {
                self.current_playlist
                    .as_ref()
                    .and_then(PlaylistSpec::restore_intent)
            });
        self.current_ready = Some(false);
        self.current_file = None;
        self.current_playlist = None;
        self.connected_intent = false;
        self.awaiting_server_playlist = true;
        self.phase = ReferencePhase::Reconnecting { attempt: retries };

        if retries > REFERENCE_MAX_RETRIES {
            self.reconnect_in_progress = false;
            self.phase = ReferencePhase::Disconnected;
            return vec![
                ClientRuntimeAction::NotifyReconnectTransition(
                    ReconnectTransitionNotification::Disconnected,
                ),
                ClientRuntimeAction::StopReconnect,
            ];
        }

        self.reconnect_in_progress = true;
        let exponent = retries.min(REFERENCE_MAX_BACKOFF_EXPONENT);
        let delay_seconds = REFERENCE_BASE_DELAY_SECONDS * 2_f64.powi(exponent as i32);
        vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::Attempting {
                    retries,
                    delay_seconds,
                },
            ),
            ClientRuntimeAction::ScheduleReconnect { delay_seconds },
        ]
    }

    fn can_apply_hello(&self) -> bool {
        self.reconnect_in_progress
    }

    fn apply_hello(&mut self, shared_playlists: bool) {
        assert!(
            self.reconnect_in_progress,
            "reference precondition: Hello requires an in-progress retry"
        );
        self.reconnect_in_progress = false;
        self.connected_intent = true;
        self.phase = ReferencePhase::Active { shared_playlists };

        self.current_ready = Some(false);
        if let Some(ready) = self.ready_snapshot.take() {
            self.current_ready = Some(ready);
            self.ready_intent = Some(ready);
        }

        self.current_file = None;
        if let Some(file) = self.file_snapshot.take() {
            self.current_file = Some(file.clone());
            self.file_intent = Some(file);
        }
    }

    fn can_apply_server_playlist(&self) -> bool {
        matches!(self.phase, ReferencePhase::Active { .. }) && self.awaiting_server_playlist
    }

    fn apply_empty_server_playlist(&mut self) {
        assert!(
            self.can_apply_server_playlist(),
            "reference precondition: initial playlist snapshot requires active Hello"
        );
        self.awaiting_server_playlist = false;
        if let Some(restore_intent) = self.playlist_snapshot.take() {
            self.playlist_intent = Some(restore_intent);
        } else {
            self.current_playlist = Some(PlaylistSpec {
                files: Vec::new(),
                index: None,
            });
        }
    }

    fn apply_non_empty_server_playlist(&mut self, playlist: PlaylistSpec) {
        assert!(
            self.can_apply_server_playlist(),
            "reference precondition: initial playlist snapshot requires active Hello"
        );
        assert!(
            !playlist.files.is_empty(),
            "reference precondition: authoritative playlist is non-empty"
        );
        self.awaiting_server_playlist = false;
        self.playlist_snapshot = None;
        self.current_playlist = Some(playlist);
    }

    fn drain_transition(&mut self) -> Vec<ClientRuntimeAction> {
        if !self.connected_intent {
            return Vec::new();
        }
        self.connected_intent = false;
        vec![ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::Connected,
        )]
    }

    fn drain_state(&mut self) -> Vec<ClientRuntimeAction> {
        let ready = self.ready_intent.take();
        let file = self.file_intent.take();
        if ready.is_none() && file.is_none() {
            return Vec::new();
        }

        let mut actions = vec![ClientRuntimeAction::NotifyReconnectTransition(
            ReconnectTransitionNotification::RestoringState,
        )];
        if let Some(ready) = ready {
            actions.push(ClientRuntimeAction::SetReady {
                ready,
                manually_initiated: false,
            });
        }
        if let Some(file) = file {
            actions.push(ClientRuntimeAction::SetFile {
                file: file.payload(),
            });
            actions.push(ClientRuntimeAction::RequestUserList);
        }
        actions
    }

    fn drain_playlist(&mut self) -> Vec<ClientRuntimeAction> {
        let ReferencePhase::Active { shared_playlists } = self.phase else {
            return Vec::new();
        };
        let Some(restore_intent) = self.playlist_intent.take() else {
            return Vec::new();
        };
        if !shared_playlists {
            return Vec::new();
        }

        let mut actions = vec![
            ClientRuntimeAction::NotifyReconnectTransition(
                ReconnectTransitionNotification::RestoringPlaylist,
            ),
            ClientRuntimeAction::SetPlaylist {
                files: restore_intent.files,
            },
        ];
        if let Some(index) = restore_intent.index {
            actions.push(ClientRuntimeAction::SetPlaylistIndex { index });
        }
        actions
    }

    fn assert_matches(&self, session: &ClientSession, context: &str) {
        match (&self.phase, session.connection_phase()) {
            (
                ReferencePhase::Active { shared_playlists },
                ConnectionPhase::Active(capabilities),
            ) => assert_eq!(
                capabilities.shared_playlists, *shared_playlists,
                "{context}: shared-playlist capability drift"
            ),
            (
                ReferencePhase::Reconnecting { attempt: expected },
                ConnectionPhase::Reconnecting { attempt: actual },
            ) => assert_eq!(actual, expected, "{context}: retry attempt drift"),
            (ReferencePhase::Disconnected, ConnectionPhase::Disconnected) => {}
            (expected, actual) => {
                panic!(
                    "{context}: connection phase drift: expected {expected:?}, actual {actual:?}"
                )
            }
        }
        assert_eq!(
            session.model.reconnect.in_progress, self.reconnect_in_progress,
            "{context}: reconnect-in-progress drift"
        );
        assert_eq!(
            session.model.reconnect.connected_intent, self.connected_intent,
            "{context}: connected notification intent drift"
        );
        assert_eq!(
            session.user_ready("alice"),
            self.current_ready,
            "{context}: local readiness projection drift"
        );
        assert_eq!(
            session.user_file_name("alice"),
            self.current_file.as_ref().map(|file| file.name.as_str()),
            "{context}: local file-name projection drift"
        );
        assert_eq!(
            session.user_file_duration("alice"),
            self.current_file
                .as_ref()
                .map(|file| f64::from(file.duration_seconds)),
            "{context}: local file-duration projection drift"
        );

        match (&self.current_playlist, session.current_room_playlist()) {
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

fn file_spec_strategy() -> impl Strategy<Value = FileSpec> {
    ("[a-z]{1,8}", 1_u64..=10_000_000, 1_u16..=7_200).prop_map(|(stem, size, duration_seconds)| {
        FileSpec {
            name: format!("{stem}.mkv"),
            size,
            duration_seconds,
        }
    })
}

fn playlist_spec_strategy() -> impl Strategy<Value = PlaylistSpec> {
    (
        prop::collection::vec("[a-z]{1,6}", 1..=4),
        prop::option::of(-2_i64..=6),
    )
        .prop_map(|(stems, index)| PlaylistSpec {
            files: stems
                .into_iter()
                .enumerate()
                .map(|(position, stem)| format!("{stem}-{position}.mkv"))
                .collect(),
            index,
        })
}

fn restore_seed_strategy() -> impl Strategy<Value = RestoreSeed> {
    (
        any::<bool>(),
        prop::option::of(file_spec_strategy()),
        playlist_spec_strategy(),
        any::<bool>(),
        any::<bool>(),
        playlist_spec_strategy(),
    )
        .prop_map(
            |(
                ready,
                file,
                playlist,
                fallback_shared_playlists,
                fallback_empty_server_playlist,
                fallback_server_playlist,
            )| RestoreSeed {
                ready,
                file,
                playlist,
                fallback_shared_playlists,
                fallback_empty_server_playlist,
                fallback_server_playlist,
            },
        )
}

fn reconnect_step_strategy() -> impl Strategy<Value = ReconnectStep> {
    prop_oneof![
        3 => (0_u8..=5).prop_map(ReconnectStep::Retry),
        3 => any::<bool>().prop_map(|shared_playlists| ReconnectStep::Hello {
            shared_playlists,
        }),
        2 => Just(ReconnectStep::EmptyServerPlaylist),
        2 => (0_u8..=15, prop::option::of(-1_i64..=3)).prop_map(
            |(variant, index)| ReconnectStep::NonEmptyServerPlaylist { variant, index }
        ),
        2 => Just(ReconnectStep::DrainTransition),
        2 => Just(ReconnectStep::DrainState),
        2 => Just(ReconnectStep::DrainPlaylist),
    ]
}

fn server_playlist(variant: u8, index: Option<i64>) -> PlaylistSpec {
    let file_count = usize::from(variant % 3) + 1;
    PlaylistSpec {
        files: (0..file_count)
            .map(|position| format!("server-{variant}-{position}.mkv"))
            .collect(),
        index,
    }
}

fn apply_json(session: &mut ClientSession, value: Value) {
    session
        .apply_message_json(&value.to_string())
        .expect("generated reconnect protocol message should apply");
}

fn session_from_seed(seed: &RestoreSeed) -> ClientSession {
    let mut session = ClientSession::default();
    session.reconnect_policy_mut().max_retries = REFERENCE_MAX_RETRIES;
    session.reconnect_policy_mut().base_delay_seconds = REFERENCE_BASE_DELAY_SECONDS;
    session.reconnect_policy_mut().max_backoff_exponent = REFERENCE_MAX_BACKOFF_EXPONENT;
    apply_json(
        &mut session,
        json!({
            "Hello": {
                "username": "alice",
                "room": {"name": "room1"},
                "version": "1.7.5",
                "features": {"sharedPlaylists": true},
            }
        }),
    );
    apply_json(
        &mut session,
        json!({
            "Set": {
                "ready": {
                    "isReady": seed.ready,
                    "username": "alice",
                }
            }
        }),
    );
    if let Some(file) = &seed.file {
        apply_json(
            &mut session,
            json!({
                "Set": {
                    "user": {
                        "alice": {
                            "room": {"name": "room1"},
                            "file": {
                                "name": file.name,
                                "size": file.size,
                                "duration": f64::from(file.duration_seconds),
                            }
                        }
                    }
                }
            }),
        );
    }
    apply_json(
        &mut session,
        json!({
            "Set": {
                "playlistChange": {
                    "files": seed.playlist.files,
                    "user": "alice",
                }
            }
        }),
    );
    if let Some(index) = seed.playlist.index {
        apply_json(
            &mut session,
            json!({
                "Set": {
                    "playlistIndex": {
                        "index": index,
                        "user": "alice",
                    }
                }
            }),
        );
    }
    session
}

fn apply_step(
    session: &mut ClientSession,
    model: &mut ReconnectReferenceModel,
    step: &ReconnectStep,
    context: &str,
) -> bool {
    match step {
        ReconnectStep::Retry(retries) => {
            let retries = u32::from(*retries);
            let expected = model.reset_for_retry(retries);
            let actual = session.runtime_actions_for_reconnect_retry(retries);
            assert_eq!(actual, expected, "{context}: retry actions drift");
        }
        ReconnectStep::Hello { shared_playlists } => {
            if !model.can_apply_hello() {
                return false;
            }
            model.apply_hello(*shared_playlists);
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
        ReconnectStep::EmptyServerPlaylist => {
            if !model.can_apply_server_playlist() {
                return false;
            }
            model.apply_empty_server_playlist();
            apply_json(session, json!({"Set": {"playlistChange": {"files": []}}}));
        }
        ReconnectStep::NonEmptyServerPlaylist { variant, index } => {
            if !model.can_apply_server_playlist() {
                return false;
            }
            let playlist = server_playlist(*variant, *index);
            model.apply_non_empty_server_playlist(playlist.clone());
            apply_json(
                session,
                json!({"Set": {"playlistChange": {"files": playlist.files}}}),
            );
            if let Some(index) = playlist.index {
                apply_json(session, json!({"Set": {"playlistIndex": {"index": index}}}));
            }
        }
        ReconnectStep::DrainTransition => {
            let expected = model.drain_transition();
            let actual = session.runtime_actions_for_reconnect_transition_if_needed();
            assert_eq!(actual, expected, "{context}: transition drain drift");
        }
        ReconnectStep::DrainState => {
            let expected = model.drain_state();
            let actual = session.runtime_actions_for_reconnect_state_restore_if_needed();
            assert_eq!(actual, expected, "{context}: state-restore drain drift");
        }
        ReconnectStep::DrainPlaylist => {
            let expected = model.drain_playlist();
            let actual = session.runtime_actions_for_reconnect_playlist_restore_if_needed();
            assert_eq!(actual, expected, "{context}: playlist-restore drain drift");
        }
    }
    model.assert_matches(session, context);
    true
}

fn finish_schedule(
    session: &mut ClientSession,
    model: &mut ReconnectReferenceModel,
    seed: &RestoreSeed,
) {
    if !model.reconnect_in_progress && !matches!(model.phase, ReferencePhase::Active { .. }) {
        assert!(apply_step(
            session,
            model,
            &ReconnectStep::Retry(0),
            "finalize retry"
        ));
    }
    if model.reconnect_in_progress {
        assert!(apply_step(
            session,
            model,
            &ReconnectStep::Hello {
                shared_playlists: seed.fallback_shared_playlists,
            },
            "finalize Hello"
        ));
    }
    if model.can_apply_server_playlist() {
        let final_snapshot = if seed.fallback_empty_server_playlist {
            ReconnectStep::EmptyServerPlaylist
        } else {
            ReconnectStep::NonEmptyServerPlaylist {
                variant: 31,
                index: seed.fallback_server_playlist.index,
            }
        };
        assert!(apply_step(
            session,
            model,
            &final_snapshot,
            "finalize server playlist"
        ));
    }
    for pass in 0..2 {
        for step in [
            ReconnectStep::DrainTransition,
            ReconnectStep::DrainState,
            ReconnectStep::DrainPlaylist,
        ] {
            assert!(apply_step(
                session,
                model,
                &step,
                &format!("final drain pass {pass}: {step:?}")
            ));
        }
    }
}

fn resolve_proptest_cases(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_PROPTEST_CASES);
    };
    let cases = raw
        .parse::<u32>()
        .map_err(|_| format!("PROPTEST_CASES must be an integer from 1 to {MAX_PROPTEST_CASES}"))?;
    if cases == 0 {
        return Err(format!(
            "PROPTEST_CASES must be an integer from 1 to {MAX_PROPTEST_CASES}"
        ));
    }
    Ok(cases.min(MAX_PROPTEST_CASES))
}

fn configured_proptest() -> ProptestConfig {
    let raw_cases = std::env::var("PROPTEST_CASES").ok();
    ProptestConfig {
        cases: resolve_proptest_cases(raw_cases.as_deref())
            .unwrap_or_else(|reason| panic!("{reason}")),
        max_shrink_iters: 20_000,
        ..ProptestConfig::default()
    }
}

#[test]
fn reconnect_proptest_case_budget_rejects_zero_and_caps_excessive_values() {
    assert_eq!(resolve_proptest_cases(None), Ok(DEFAULT_PROPTEST_CASES));
    assert_eq!(resolve_proptest_cases(Some("2048")), Ok(2_048));
    assert_eq!(
        resolve_proptest_cases(Some(&u32::MAX.to_string())),
        Ok(MAX_PROPTEST_CASES)
    );
    for invalid in ["", "0", "-1", "not-a-number"] {
        assert!(
            resolve_proptest_cases(Some(invalid)).is_err(),
            "{invalid:?} must not silently weaken the property budget"
        );
    }
}

#[test]
fn declared_reconnect_schedule_vocabulary_exercises_every_transition_kind() {
    let seed = RestoreSeed {
        ready: true,
        file: Some(FileSpec {
            name: "movie.mkv".to_owned(),
            size: 123_456,
            duration_seconds: 95,
        }),
        playlist: PlaylistSpec {
            files: vec!["episode-1.mkv".to_owned(), "episode-2.mkv".to_owned()],
            index: Some(1),
        },
        fallback_shared_playlists: true,
        fallback_empty_server_playlist: true,
        fallback_server_playlist: PlaylistSpec {
            files: vec!["server-fallback.mkv".to_owned()],
            index: Some(0),
        },
    };
    let mut session = session_from_seed(&seed);
    let mut model = ReconnectReferenceModel::from_seed(&seed);
    let schedule = [
        ReconnectStep::Retry(0),
        ReconnectStep::DrainTransition,
        ReconnectStep::DrainState,
        ReconnectStep::DrainPlaylist,
        ReconnectStep::Hello {
            shared_playlists: true,
        },
        ReconnectStep::EmptyServerPlaylist,
        ReconnectStep::DrainTransition,
        ReconnectStep::DrainState,
        ReconnectStep::DrainPlaylist,
        ReconnectStep::Retry(1),
        ReconnectStep::Hello {
            shared_playlists: false,
        },
        ReconnectStep::NonEmptyServerPlaylist {
            variant: 7,
            index: Some(0),
        },
    ];
    let mut executed = BTreeSet::new();
    for (index, step) in schedule.iter().enumerate() {
        if apply_step(
            &mut session,
            &mut model,
            step,
            &format!("vocabulary step {index}: {step:?}"),
        ) {
            executed.insert(step.kind());
        }
    }
    assert_eq!(
        executed,
        ALL_RECONNECT_STEP_KINDS.iter().copied().collect(),
        "the declared reconnect generator vocabulary must stay executable"
    );
}

proptest! {
    #![proptest_config(configured_proptest())]

    #[test]
    fn generated_reconnect_restore_histories_match_reference_model(
        seed in restore_seed_strategy(),
        steps in prop::collection::vec(reconnect_step_strategy(), 1..=64),
    ) {
        let mut session = session_from_seed(&seed);
        let mut model = ReconnectReferenceModel::from_seed(&seed);
        let initial_retry = ReconnectStep::Retry(0);
        assert!(apply_step(
            &mut session,
            &mut model,
            &initial_retry,
            "mandatory initial retry",
        ));

        for (index, step) in steps.iter().enumerate() {
            let _ = apply_step(
                &mut session,
                &mut model,
                step,
                &format!("generated step {index}: {step:?}"),
            );
        }
        finish_schedule(&mut session, &mut model, &seed);
    }
}

fn session_with_restorable_playlist() -> ClientSession {
    let seed = RestoreSeed {
        ready: false,
        file: None,
        playlist: PlaylistSpec {
            files: vec!["episode-1.mkv".to_owned(), "episode-2.mkv".to_owned()],
            index: Some(1),
        },
        fallback_shared_playlists: true,
        fallback_empty_server_playlist: true,
        fallback_server_playlist: PlaylistSpec {
            files: vec!["server.mkv".to_owned()],
            index: Some(0),
        },
    };
    session_from_seed(&seed)
}

fn reconnect_to_empty_shared_playlist(session: &mut ClientSession) {
    session.reset_sync_state_for_reconnect();
    apply_json(
        session,
        json!({
            "Hello": {
                "username": "alice",
                "room": {"name": "room1"},
                "version": "1.7.5",
                "features": {"sharedPlaylists": true},
            }
        }),
    );
    apply_json(session, json!({"Set": {"playlistChange": {"files": []}}}));
}

#[test]
#[should_panic(expected = "unacknowledged playlist restore must survive a second disconnect")]
fn known_defect_unacknowledged_playlist_restore_is_lost_on_second_disconnect() {
    let mut session = session_with_restorable_playlist();
    reconnect_to_empty_shared_playlist(&mut session);
    assert!(
        !session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "precondition: first reconnect should emit the playlist restore"
    );

    reconnect_to_empty_shared_playlist(&mut session);
    assert!(
        !session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "unacknowledged playlist restore must survive a second disconnect"
    );
}

#[test]
#[should_panic(expected = "authoritative playlist update must cancel an armed restore")]
fn known_defect_authoritative_playlist_does_not_cancel_armed_restore() {
    let mut session = session_with_restorable_playlist();
    reconnect_to_empty_shared_playlist(&mut session);
    apply_json(
        &mut session,
        json!({
            "Set": {
                "playlistChange": {
                    "files": ["server-authoritative.mkv"],
                }
            }
        }),
    );

    assert!(
        session
            .runtime_actions_for_reconnect_playlist_restore_if_needed()
            .is_empty(),
        "authoritative playlist update must cancel an armed restore"
    );
}
