pub(in crate::app) mod barrier;

use super::GuiClientSession;

/// Read-only observations at the session boundary. Tests still execute the real
/// client session; this hook cannot replace an operation or its result.
pub(in crate::app) enum SessionObservation<'a> {
    PlaybackObserved {
        paused: Option<bool>,
        position: Option<f64>,
    },
    SeekRequested {
        position: f64,
        published: bool,
    },
    TransportObserved(&'a sorotte_player_api::PlayerTransportTelemetryUpdate),
    EndOfFile,
    Availability(sorotte_client_core::ExternalPlayerAvailability),
    AttachmentReset,
    MediaPrepared(sorotte_client_core::MediaLoadIntent),
    RecoveryInterrupted,
    SeekWaitRenewed,
    PauseIntentStaged(bool),
    PlaybackStatePublication,
    PlaylistAdvance,
    PauseRequested(bool),
}

pub(in crate::app::runtime_stack) type SessionObserver =
    Box<dyn FnMut(SessionObservation<'_>) + Send>;

pub(in crate::app) enum SessionFailure {
    TelemetryOnCall(usize),
    UnpauseFinalization,
    SeekPublications(usize),
}

pub(in crate::app::runtime_stack) enum SessionFailurePoint {
    Telemetry,
    UnpauseFinalization,
    SeekPublication,
}

pub(in crate::app) fn active_session() -> GuiClientSession {
    active_session_in_room("room1")
}

pub(in crate::app) fn active_session_in_room(room: &str) -> GuiClientSession {
    let mut session = GuiClientSession::new("alice", room).unwrap();
    session.deliver_outbound_protocol_lines().unwrap();
    session
        .apply_message_json(
            &serde_json::json!({"Hello":{
                "username":"alice", "room":{"name":room}, "version":"1.7.5",
                "features":{"sharedPlaylists":true,"chat":true,"readiness":true,
                    "setOthersReadiness":true,"managedRooms":true,"mediaMatch":true}
            }})
            .to_string(),
        )
        .unwrap();
    session.deliver_outbound_protocol_lines().unwrap();
    session
}

pub(in crate::app) fn session_with_media_target(target: String) -> GuiClientSession {
    let mut session = active_session();
    session
        .apply_message_json(
            &serde_json::json!({"Set": {
                "playlistChange": {"files": [target], "user": "bob"},
                "playlistIndex": {"index": 0, "user": "bob"}
            }})
            .to_string(),
        )
        .unwrap();
    session
}

pub(in crate::app) fn empty_media_signature() -> sorotte_media_match::MediaMatchWireSignature {
    sorotte_media_match::MediaMatchWireSignature {
        schema: sorotte_media_match::MEDIA_MATCH_WIRE_SCHEMA_V3.to_owned(),
        profiles: vec![sorotte_media_match::MediaMatchWireProfile {
            profile: sorotte_media_match::MEDIA_MATCH_V3_PROFILE_LABEL.to_owned(),
            algorithm_version: sorotte_media_match::MEDIA_MATCH_ANCHOR_VERSION,
            duration_ms: None,
            audio: None,
        }],
    }
}

pub(in crate::app) fn session_with_peer_files(
    peers: Vec<sorotte_client_core::ClientMediaMatchPeerFileState>,
) -> GuiClientSession {
    let mut session = active_session();
    for peer in peers {
        let file = peer.has_file.then(|| {
            serde_json::json!({
                "name": peer.file_name,
                "size": peer.file_size.as_ref().map(|size| size.to_json_value()),
                "duration": peer.file_duration,
                "mediaMatch": peer.media_match_signature,
            })
        });
        session
            .apply_message_json(
                &serde_json::json!({
                    "Set": {"user": {peer.username: {"room": {"name": "room1"}, "file": file}}}
                })
                .to_string(),
            )
            .unwrap();
    }
    session
}

impl GuiClientSession {
    pub(in crate::app) fn with_failure(mut self, failure: SessionFailure) -> Self {
        self.test_failure = Some(failure);
        self
    }

    pub(in crate::app::runtime_stack) fn take_test_failure(
        &mut self,
        point: SessionFailurePoint,
    ) -> bool {
        match (self.test_failure.as_mut(), point) {
            (Some(SessionFailure::TelemetryOnCall(remaining)), SessionFailurePoint::Telemetry) => {
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    self.test_failure = None;
                    return true;
                }
            }
            (
                Some(SessionFailure::UnpauseFinalization),
                SessionFailurePoint::UnpauseFinalization,
            ) => {
                self.test_failure = None;
                return true;
            }
            (
                Some(SessionFailure::SeekPublications(remaining)),
                SessionFailurePoint::SeekPublication,
            ) if *remaining > 0 => {
                *remaining -= 1;
                return true;
            }
            _ => {}
        }
        false
    }

    pub(in crate::app) fn with_observer(
        mut self,
        observer: impl FnMut(SessionObservation<'_>) + Send + 'static,
    ) -> Self {
        self.test_observer = Some(Box::new(observer));
        self
    }

    pub(in crate::app::runtime_stack) fn observe_for_test(
        &mut self,
        event: SessionObservation<'_>,
    ) {
        if let Some(observer) = self.test_observer.as_mut() {
            observer(event);
        }
    }

    /// Capture completed writes through the production staging and receipt API.
    pub(in crate::app) fn deliver_outbound_protocol_lines(
        &mut self,
    ) -> Result<Vec<String>, String> {
        let mut delivered = Vec::new();
        while let Some(frame) = self.begin_outbound_protocol_delivery()? {
            delivered.push(self.acknowledge_outbound_protocol_delivery(frame.token())?);
        }
        Ok(delivered)
    }
}
