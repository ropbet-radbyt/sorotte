use super::super::widget_tree::GuiStatusTone;
use super::super::{
    GuiDroppedFilesTarget, GuiPendingOperationKind, GuiPlayerSetupIssue, GuiPlayerSetupIssueKind,
    GuiPlayerSetupRuntimeSnapshot, GuiSeekPreparationDegradedReason, GuiSeekPreparationPhase,
    GuiSeekPreparationRuntimeSnapshot, GuiSeekPreparationState,
    MainWindowParticipantStatusFreshness, MainWindowParticipantStatusPresentation,
    MainWindowParticipantStatusReport, MainWindowRoomPlaybackIntent, MainWindowRuntimeChatSnapshot,
    MainWindowRuntimeRoomSnapshot, MainWindowRuntimeSnapshot, MainWindowRuntimeUserSnapshot,
};
use sorotte_client_app::app_boundary::readiness::{
    ParticipantReadinessPresentation, PendingReadinessIntentPresentation,
    ReadinessPresentationProtocol,
};
use sorotte_client_core::ClientParticipantStatusView;
use sorotte_protocol::{
    ParticipantPlaybackPhase, ParticipantPlaybackScope, ParticipantPlayerConnection,
    ParticipantStatusAvailability, ParticipantStatusCorrelation, ParticipantStatusView,
    ParticipantTimelineKind, RecoveryStage, StartParticipationRole, TechnicalBlockCause,
    TechnicalPlayabilityPhase, UserReadinessIntent,
};

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) enum GuiSemanticStep {
    ApplyMainWindowRuntimeSnapshot(MainWindowRuntimeSnapshot),
    ApplyMainWindowReadinessPresentation(ParticipantReadinessPresentation),
    ApplyMainWindowRoomPlaybackIntent(MainWindowRoomPlaybackIntent),
    ApplyMainWindowParticipantStatus {
        username: String,
        status: MainWindowParticipantStatusPresentation,
        start_barrier_status: Option<String>,
    },
    ApplyMainWindowPlaylistSelection(Option<usize>),
    ApplyPlayerSetupRuntimeSnapshot(GuiPlayerSetupRuntimeSnapshot),
    ApplySeekPreparationRuntimeSnapshot(GuiSeekPreparationRuntimeSnapshot),
    OpenMediaFiles(Vec<String>),
    DropMediaFiles {
        target: GuiDroppedFilesTarget,
        paths: Vec<String>,
    },
    AddMediaSearchDirectory(String),
    PushChatMessage {
        sender: String,
        message: String,
    },
    Activate(String),
    EnterText {
        widget_id: String,
        value: String,
        submit: bool,
    },
    AssertWidgetLabel {
        widget_id: String,
        label: String,
    },
    AssertWidgetValue {
        widget_id: String,
        value: Option<String>,
    },
    AssertWidgetSelected {
        widget_id: String,
        selected: bool,
    },
    AssertWidgetEnabled {
        widget_id: String,
        enabled: bool,
    },
    AssertWidgetTone {
        widget_id: String,
        tone: GuiStatusTone,
    },
    AssertWidgetTooltipContains {
        widget_id: String,
        text: String,
    },
    AssertPending {
        pending: Option<GuiPendingOperationKind>,
    },
    CompletePending,
    CompletePendingRuntime,
    CancelPending,
    CloseModal,
    ClearNotifications,
}

impl GuiSemanticStep {
    pub(in crate::app) fn activate(widget_id: &str) -> Self {
        Self::Activate(widget_id.to_owned())
    }

    pub(in crate::app) fn enter_text(widget_id: &str, value: &str, submit: bool) -> Self {
        Self::EnterText {
            widget_id: widget_id.to_owned(),
            value: value.to_owned(),
            submit,
        }
    }

    pub(in crate::app) fn assert_widget_label(widget_id: &str, label: &str) -> Self {
        Self::AssertWidgetLabel {
            widget_id: widget_id.to_owned(),
            label: label.to_owned(),
        }
    }

    pub(in crate::app) fn assert_widget_value(widget_id: &str, value: Option<&str>) -> Self {
        Self::AssertWidgetValue {
            widget_id: widget_id.to_owned(),
            value: value.map(str::to_owned),
        }
    }

    pub(in crate::app) fn assert_widget_selected(widget_id: &str, selected: bool) -> Self {
        Self::AssertWidgetSelected {
            widget_id: widget_id.to_owned(),
            selected,
        }
    }

    pub(in crate::app) fn assert_widget_enabled(widget_id: &str, enabled: bool) -> Self {
        Self::AssertWidgetEnabled {
            widget_id: widget_id.to_owned(),
            enabled,
        }
    }

    pub(in crate::app) fn assert_widget_tone(widget_id: &str, tone: GuiStatusTone) -> Self {
        Self::AssertWidgetTone {
            widget_id: widget_id.to_owned(),
            tone,
        }
    }

    pub(in crate::app) fn assert_widget_tooltip_contains(widget_id: &str, text: &str) -> Self {
        Self::AssertWidgetTooltipContains {
            widget_id: widget_id.to_owned(),
            text: text.to_owned(),
        }
    }

    pub(in crate::app) fn assert_pending(pending: Option<GuiPendingOperationKind>) -> Self {
        Self::AssertPending { pending }
    }

    fn parse_index(token: &str) -> Result<usize, String> {
        token
            .parse::<usize>()
            .map_err(|_| format!("expected a non-negative index, got {token:?}"))
    }

    fn parse_optional_index(token: &str) -> Result<Option<usize>, String> {
        if token == "none" {
            Ok(None)
        } else {
            Self::parse_index(token).map(Some)
        }
    }

    fn parse_bool(token: &str) -> Result<bool, String> {
        match token {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("expected boolean 'true' or 'false', got {token:?}")),
        }
    }

    fn parse_status_tone(token: &str) -> Result<GuiStatusTone, String> {
        match token {
            "danger" => Ok(GuiStatusTone::Danger),
            "warning" => Ok(GuiStatusTone::Warning),
            "success" => Ok(GuiStatusTone::Success),
            "muted" => Ok(GuiStatusTone::Muted),
            _ => Err(format!(
                "expected status tone 'danger', 'warning', 'success', or 'muted', got {token:?}"
            )),
        }
    }

    fn parse_u64(token: &str, label: &str) -> Result<u64, String> {
        token
            .parse::<u64>()
            .map_err(|_| format!("expected {label} to be a non-negative integer, got {token:?}"))
    }

    fn parse_readiness_intent(token: &str) -> Result<UserReadinessIntent, String> {
        match token {
            "ready" => Ok(UserReadinessIntent::Ready),
            "not-ready" => Ok(UserReadinessIntent::NotReady),
            _ => Err(format!(
                "expected readiness intent 'ready' or 'not-ready', got {token:?}"
            )),
        }
    }

    fn parse_optional_readiness_intent(token: &str) -> Result<Option<UserReadinessIntent>, String> {
        if token == "none" {
            Ok(None)
        } else {
            Self::parse_readiness_intent(token).map(Some)
        }
    }

    fn parse_technical_phase(token: &str) -> Result<TechnicalPlayabilityPhase, String> {
        match token {
            "unknown" => Ok(TechnicalPlayabilityPhase::Unknown),
            "preparing" => Ok(TechnicalPlayabilityPhase::Preparing),
            "playable" => Ok(TechnicalPlayabilityPhase::Playable),
            "temporarily-blocked" => Ok(TechnicalPlayabilityPhase::TemporarilyBlocked),
            "terminally-blocked" => Ok(TechnicalPlayabilityPhase::TerminallyBlocked),
            _ => Err(format!("unsupported technical readiness phase {token:?}")),
        }
    }

    fn parse_optional_technical_reason(token: &str) -> Result<Option<TechnicalBlockCause>, String> {
        let reason = match token {
            "none" => return Ok(None),
            "loading" => TechnicalBlockCause::Loading,
            "seeking" => TechnicalBlockCause::Seeking,
            "prebuffering" => TechnicalBlockCause::Prebuffering,
            "rebuffering" => TechnicalBlockCause::Rebuffering,
            "cache-pause" => TechnicalBlockCause::CachePause,
            "transport-refresh" => TechnicalBlockCause::TransportRefresh,
            "recovery" => TechnicalBlockCause::Recovery,
            "player-failure" => TechnicalBlockCause::PlayerFailure,
            "adapter-failure" => TechnicalBlockCause::AdapterFailure,
            "recovery-exhausted" => TechnicalBlockCause::RecoveryExhausted,
            _ => return Err(format!("unsupported technical readiness reason {token:?}")),
        };
        Ok(Some(reason))
    }

    fn parse_optional_recovery_stage(token: &str) -> Result<Option<RecoveryStage>, String> {
        let stage = match token {
            "none" => return Ok(None),
            "not-started" => RecoveryStage::NotStarted,
            "waiting" => RecoveryStage::Waiting,
            "retrying" => RecoveryStage::Retrying,
            "reloading-media" => RecoveryStage::ReloadingMedia,
            "restarting-player" => RecoveryStage::RestartingPlayer,
            "replacing-adapter" => RecoveryStage::ReplacingAdapter,
            _ => return Err(format!("unsupported readiness recovery stage {token:?}")),
        };
        Ok(Some(stage))
    }

    fn parse_optional_text(token: &str) -> Option<String> {
        (token != "none").then(|| token.to_owned())
    }

    fn parse_f64(token: &str, field: &str) -> Result<f64, String> {
        token
            .parse::<f64>()
            .map_err(|_| format!("expected a number for {field}, got {token:?}"))
    }

    fn parse_optional_f64(token: &str, field: &str) -> Result<Option<f64>, String> {
        if token == "none" {
            Ok(None)
        } else {
            Self::parse_f64(token, field).map(Some)
        }
    }

    fn parse_optional_bool(token: &str) -> Result<Option<bool>, String> {
        if token == "none" {
            Ok(None)
        } else {
            Self::parse_bool(token).map(Some)
        }
    }

    fn parse_optional_u64(token: &str, field: &str) -> Result<Option<u64>, String> {
        if token == "none" {
            Ok(None)
        } else {
            Self::parse_u64(token, field).map(Some)
        }
    }

    fn parse_member_player_availability(
        token: &str,
    ) -> Result<ParticipantPlayerConnection, String> {
        match token {
            "unavailable" => Ok(ParticipantPlayerConnection::Unavailable),
            "starting" | "connecting" => Ok(ParticipantPlayerConnection::Starting),
            "connected" => Ok(ParticipantPlayerConnection::Connected),
            "disconnected" => Ok(ParticipantPlayerConnection::Disconnected),
            "telemetry-unavailable" => Ok(ParticipantPlayerConnection::Unavailable),
            "failed" => Ok(ParticipantPlayerConnection::Failed),
            _ => Err(format!("unsupported member player availability {token:?}")),
        }
    }

    fn parse_member_playback_phase(token: &str) -> Result<ParticipantPlaybackPhase, String> {
        match token {
            "unknown" => Ok(ParticipantPlaybackPhase::Unknown),
            "empty" => Ok(ParticipantPlaybackPhase::Empty),
            "loading" => Ok(ParticipantPlaybackPhase::Loading),
            "prebuffering" => Ok(ParticipantPlaybackPhase::Prebuffering),
            "ready-paused" => Ok(ParticipantPlaybackPhase::ReadyPaused),
            "waiting-for-room" => Ok(ParticipantPlaybackPhase::ReadyPaused),
            "starting" => Ok(ParticipantPlaybackPhase::Loading),
            "playing" => Ok(ParticipantPlaybackPhase::Playing),
            "rebuffering" => Ok(ParticipantPlaybackPhase::Rebuffering),
            "seeking" => Ok(ParticipantPlaybackPhase::Seeking),
            "catching-up" => Ok(ParticipantPlaybackPhase::Playing),
            "recovering" => Ok(ParticipantPlaybackPhase::Seeking),
            "degraded" => Ok(ParticipantPlaybackPhase::Unknown),
            "ended" => Ok(ParticipantPlaybackPhase::Ended),
            "failed" => Ok(ParticipantPlaybackPhase::Failed),
            _ => Err(format!("unsupported member playback phase {token:?}")),
        }
    }

    fn parse_participant_status_correlation(
        token: &str,
    ) -> Result<Option<ParticipantStatusCorrelation>, String> {
        match token {
            "none" => Ok(None),
            "exact" => Ok(Some(ParticipantStatusCorrelation::Exact)),
            "uncorrelated" => Ok(Some(ParticipantStatusCorrelation::Uncorrelated)),
            "superseded" => Ok(Some(ParticipantStatusCorrelation::Superseded)),
            _ => Err(format!(
                "unsupported participant status correlation {token:?}"
            )),
        }
    }

    fn parse_seek_preparation_phase(token: &str) -> Result<GuiSeekPreparationPhase, String> {
        match token {
            "seeking" => Ok(GuiSeekPreparationPhase::Seeking),
            "fetching" => Ok(GuiSeekPreparationPhase::Fetching),
            "refilling" => Ok(GuiSeekPreparationPhase::Refilling),
            "ready-to-join" => Ok(GuiSeekPreparationPhase::ReadyToJoin),
            "catching-up" => Ok(GuiSeekPreparationPhase::CatchingUp),
            _ => Err(format!("unknown seek-preparation phase {token:?}")),
        }
    }

    fn parse_seek_preparation_degraded_reason(
        token: &str,
    ) -> Result<GuiSeekPreparationDegradedReason, String> {
        match token {
            "non-seekable" => Ok(GuiSeekPreparationDegradedReason::NonSeekable),
            "outside-live-window" => Ok(GuiSeekPreparationDegradedReason::OutsideLiveWindow),
            "timed-out" => Ok(GuiSeekPreparationDegradedReason::TimedOut),
            "timeline-window-unavailable" => {
                Ok(GuiSeekPreparationDegradedReason::TimelineWindowUnavailable)
            }
            "transport-failed" => Ok(GuiSeekPreparationDegradedReason::TransportFailed),
            "convergence-degraded" => Ok(GuiSeekPreparationDegradedReason::ConvergenceDegraded),
            _ => Err(format!(
                "unknown seek-preparation degraded reason {token:?}"
            )),
        }
    }

    fn parse_drop_target(token: &str) -> Result<GuiDroppedFilesTarget, String> {
        GuiDroppedFilesTarget::parse(token)
    }

    fn decode_text_token(token: &str) -> String {
        let mut decoded = String::new();
        let mut characters = token.chars();
        while let Some(character) = characters.next() {
            if character != '\\' {
                decoded.push(character);
                continue;
            }
            match characters.next() {
                Some('n') => decoded.push('\n'),
                Some('r') => decoded.push('\r'),
                Some('t') => decoded.push('\t'),
                Some('\\') => decoded.push('\\'),
                Some(other) => {
                    decoded.push('\\');
                    decoded.push(other);
                }
                None => decoded.push('\\'),
            }
        }
        decoded
    }

    fn parse_optional_value(token: &str) -> Option<String> {
        match token {
            "<none>" => None,
            "<empty>" => Some(String::new()),
            _ => Some(Self::decode_text_token(token)),
        }
    }

    fn parse_pending(token: &str) -> Result<Option<GuiPendingOperationKind>, String> {
        let pending = match token {
            "none" => return Ok(None),
            "save-configuration" => GuiPendingOperationKind::SaveConfiguration,
            "discard-configuration-changes" => GuiPendingOperationKind::DiscardConfigurationChanges,
            "reload-configuration" => GuiPendingOperationKind::ReloadConfiguration,
            "clear-gui-data" => GuiPendingOperationKind::ClearGuiData,
            "change-config-storage-root" => GuiPendingOperationKind::ChangeConfigStorageRoot,
            "connect-saved-server" => GuiPendingOperationKind::ConnectSavedServer,
            "disconnect-session" => GuiPendingOperationKind::DisconnectSession,
            "connect-public-server" => GuiPendingOperationKind::ConnectPublicServer,
            "refresh-public-servers" => GuiPendingOperationKind::RefreshPublicServers,
            "search-missing-media" => GuiPendingOperationKind::SearchMissingMedia,
            "pause-playback" => GuiPendingOperationKind::SetPlaybackPause(true),
            "resume-playback" => GuiPendingOperationKind::SetPlaybackPause(false),
            "toggle-playback-pause" => GuiPendingOperationKind::TogglePlaybackPause,
            "send-chat-message" => GuiPendingOperationKind::SendChatMessage,
            _ => return Err(format!("unknown pending-operation label {token:?}")),
        };
        Ok(Some(pending))
    }

    fn parse_player_setup_issue_kind(token: &str) -> Result<GuiPlayerSetupIssueKind, String> {
        match token {
            "not-configured" => Ok(GuiPlayerSetupIssueKind::NotConfigured),
            "unsupported-player" => Ok(GuiPlayerSetupIssueKind::UnsupportedConfiguredPlayer),
            "missing-binary" => Ok(GuiPlayerSetupIssueKind::MissingBinary),
            "launch-failed" => Ok(GuiPlayerSetupIssueKind::LaunchFailed),
            "ipc-attach-failed" => Ok(GuiPlayerSetupIssueKind::IpcAttachFailed),
            "exited-after-launch" => Ok(GuiPlayerSetupIssueKind::ExitedAfterLaunch),
            "player-settings-degraded" => Ok(GuiPlayerSetupIssueKind::PlayerSettingsDegraded),
            "bridge-degraded" => Ok(GuiPlayerSetupIssueKind::BridgeDegraded),
            _ => Err(format!("unknown player-setup issue label {token:?}")),
        }
    }

    fn split_list_token(token: &str) -> Vec<&str> {
        if token == "<none>" {
            Vec::new()
        } else {
            token.split('|').collect()
        }
    }

    fn parse_runtime_users(token: &str) -> Result<Vec<MainWindowRuntimeUserSnapshot>, String> {
        Self::split_list_token(token)
            .into_iter()
            .map(|entry| {
                let mut fields = entry.splitn(4, ',');
                let username = fields
                    .next()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("runtime user entry {entry:?} is missing a username"))?;
                let is_self =
                    Self::parse_bool(fields.next().ok_or_else(|| {
                        format!("runtime user entry {entry:?} is missing is_self")
                    })?)?;
                let is_ready =
                    Self::parse_bool(fields.next().ok_or_else(|| {
                        format!("runtime user entry {entry:?} is missing is_ready")
                    })?)?;
                let is_controller = Self::parse_bool(fields.next().ok_or_else(|| {
                    format!("runtime user entry {entry:?} is missing is_controller")
                })?)?;
                Ok(MainWindowRuntimeUserSnapshot {
                    username: username.to_owned(),
                    room_name: String::new(),
                    is_self,
                    is_ready,
                    is_controller,
                    has_file: false,
                    file_name: None,
                    file_size_label: String::new(),
                    file_duration_label: String::new(),
                    file_is_url: false,
                    file_is_trusted: true,
                    filename_differs: false,
                    filesize_differs: false,
                    fileduration_differs: false,
                    participant_status: MainWindowParticipantStatusPresentation::Unavailable,
                    start_barrier_status: None,
                })
            })
            .collect()
    }

    fn parse_runtime_chat_rows(token: &str) -> Result<Vec<MainWindowRuntimeChatSnapshot>, String> {
        Self::split_list_token(token)
            .into_iter()
            .map(|entry| {
                let (sender, message) = entry.split_once('>').ok_or_else(|| {
                    format!("runtime chat entry {entry:?} must use 'sender>message' formatting")
                })?;
                if sender.is_empty() {
                    return Err(format!("runtime chat entry {entry:?} is missing a sender"));
                }
                Ok(MainWindowRuntimeChatSnapshot {
                    sender: sender.to_owned(),
                    message: message.to_owned(),
                })
            })
            .collect()
    }

    fn parse_main_window_runtime_snapshot<'a, I>(
        mut fields: I,
    ) -> Result<MainWindowRuntimeSnapshot, String>
    where
        I: Iterator<Item = &'a str>,
    {
        let room_name = fields
            .next()
            .ok_or_else(|| "apply-main-window-runtime requires a room name".to_owned())?
            .to_owned();
        let shared_playlist_enabled = Self::parse_bool(fields.next().ok_or_else(|| {
            "apply-main-window-runtime requires shared_playlist_enabled".to_owned()
        })?)?;
        let playback_paused = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires playback_paused".to_owned())?,
        )?;
        let autoplay_active = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires autoplay_active".to_owned())?,
        )?;
        let can_toggle_pause =
            Self::parse_bool(fields.next().ok_or_else(|| {
                "apply-main-window-runtime requires can_toggle_pause".to_owned()
            })?)?;
        let can_seek = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires can_seek".to_owned())?,
        )?;
        let can_set_ready = Self::parse_bool(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires can_set_ready".to_owned())?,
        )?;
        let can_manage_playlist =
            Self::parse_bool(fields.next().ok_or_else(|| {
                "apply-main-window-runtime requires can_manage_playlist".to_owned()
            })?)?;
        let users = Self::parse_runtime_users(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires users".to_owned())?,
        )?
        .into_iter()
        .map(|mut user| {
            if user.room_name.is_empty() {
                user.room_name = room_name.clone();
            }
            user
        })
        .collect::<Vec<_>>();
        let playlist = Self::split_list_token(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires playlist".to_owned())?,
        )
        .into_iter()
        .map(str::to_owned)
        .collect();
        let chat = Self::parse_runtime_chat_rows(
            fields
                .next()
                .ok_or_else(|| "apply-main-window-runtime requires chat rows".to_owned())?,
        )?;
        if fields.next().is_some() {
            return Err("apply-main-window-runtime accepts exactly eleven arguments".to_owned());
        }
        let room_row_name = room_name.clone();
        Ok(MainWindowRuntimeSnapshot {
            room_name,
            shared_playlist_enabled,
            controlled_room_active: false,
            hide_empty_rooms: false,
            rooms: vec![MainWindowRuntimeRoomSnapshot {
                room_name: room_row_name,
                is_controlled: false,
                has_named_users: !users.is_empty(),
            }],
            users,
            playlist,
            chat,
            can_toggle_pause,
            can_seek,
            can_set_ready,
            can_manage_playlist,
            playback_paused,
            autoplay_active,
            ..Default::default()
        })
    }

    fn from_script_line(line: &str) -> Result<Option<Self>, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return Ok(None);
        }

        let mut fields = trimmed.split('\t');
        let command = fields
            .next()
            .ok_or_else(|| "script line is missing a command".to_owned())?;

        let step = match command {
            "apply-main-window-runtime" => Self::ApplyMainWindowRuntimeSnapshot(
                Self::parse_main_window_runtime_snapshot(fields)?,
            ),
            "apply-main-window-readiness-v2" => {
                let username = fields
                    .next()
                    .ok_or_else(|| "apply-main-window-readiness-v2 requires a username".to_owned())?
                    .to_owned();
                let canonical_user_intent =
                    Self::parse_readiness_intent(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires canonical intent".to_owned()
                    })?)?;
                let technical_phase =
                    Self::parse_technical_phase(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires technical phase".to_owned()
                    })?)?;
                let technical_reason =
                    Self::parse_optional_technical_reason(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires technical reason or 'none'"
                            .to_owned()
                    })?)?;
                let recovery_stage =
                    Self::parse_optional_recovery_stage(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires recovery stage or 'none'"
                            .to_owned()
                    })?)?;
                let room_ready = Self::parse_bool(fields.next().ok_or_else(|| {
                    "apply-main-window-readiness-v2 requires room_ready".to_owned()
                })?)?;
                let start_eligible = Self::parse_bool(fields.next().ok_or_else(|| {
                    "apply-main-window-readiness-v2 requires start_eligible".to_owned()
                })?)?;
                let membership_epoch = Self::parse_u64(
                    fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires membership epoch".to_owned()
                    })?,
                    "membership epoch",
                )?;
                let room_readiness_revision = Self::parse_u64(
                    fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires room revision".to_owned()
                    })?,
                    "room revision",
                )?;
                let user_intent_revision = Self::parse_u64(
                    fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires intent revision".to_owned()
                    })?,
                    "intent revision",
                )?;
                let pending_desired =
                    Self::parse_optional_readiness_intent(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires pending intent or 'none'"
                            .to_owned()
                    })?)?;
                let pending_operation_id =
                    Self::parse_optional_text(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires pending operation or 'none'"
                            .to_owned()
                    })?);
                let accepted_operation_id =
                    Self::parse_optional_text(fields.next().ok_or_else(|| {
                        "apply-main-window-readiness-v2 requires accepted operation or 'none'"
                            .to_owned()
                    })?);
                if fields.next().is_some() {
                    return Err(
                        "apply-main-window-readiness-v2 accepts exactly thirteen arguments"
                            .to_owned(),
                    );
                }
                let pending = match (pending_desired, pending_operation_id) {
                    (Some(desired), Some(operation_id)) => {
                        Some(PendingReadinessIntentPresentation {
                            operation_id,
                            request_nonce: user_intent_revision.saturating_add(1),
                            membership_epoch,
                            desired,
                        })
                    }
                    (None, None) => None,
                    _ => {
                        return Err(
                            "pending readiness intent and operation must both be set or both be 'none'"
                                .to_owned(),
                        );
                    }
                };
                Self::ApplyMainWindowReadinessPresentation(ParticipantReadinessPresentation {
                    protocol: ReadinessPresentationProtocol::V2,
                    username,
                    canonical_user_intent,
                    technical_phase: Some(technical_phase),
                    technical_reason,
                    recovery_stage,
                    room_ready,
                    start_eligible: Some(start_eligible),
                    membership_epoch: Some(membership_epoch),
                    room_readiness_revision: Some(room_readiness_revision),
                    user_intent_revision: Some(user_intent_revision),
                    participation_role: Some(StartParticipationRole::Required),
                    mixed_readiness_policy: None,
                    start_gate_phase: None,
                    pending,
                    accepted_operation_id,
                })
            }
            "apply-main-window-room-intent" => {
                let paused = match fields.next().ok_or_else(|| {
                    "apply-main-window-room-intent requires playing, paused, or unavailable"
                        .to_owned()
                })? {
                    "playing" => Some(false),
                    "paused" => Some(true),
                    "unavailable" => None,
                    token => {
                        return Err(format!("unsupported room playback intent {token:?}"));
                    }
                };
                let position_seconds = Self::parse_optional_f64(
                    fields.next().ok_or_else(|| {
                        "apply-main-window-room-intent requires a position or 'none'".to_owned()
                    })?,
                    "room position",
                )?;
                let set_by = Self::parse_optional_text(fields.next().ok_or_else(|| {
                    "apply-main-window-room-intent requires set_by or 'none'".to_owned()
                })?);
                let authority = Self::parse_optional_text(fields.next().ok_or_else(|| {
                    "apply-main-window-room-intent requires authority or 'none'".to_owned()
                })?);
                let start_gate = Self::parse_optional_text(fields.next().ok_or_else(|| {
                    "apply-main-window-room-intent requires start gate or 'none'".to_owned()
                })?);
                let summary_fields = fields.collect::<Vec<_>>();
                let (participant_count, maximum_observed_drift_seconds, buffering_participants) =
                    match summary_fields.as_slice() {
                        [] => (0, None, Vec::new()),
                        [participant_count, maximum_drift, buffering_participants] => {
                            let participant_count = participant_count.parse::<usize>().map_err(
                                |_| {
                                    format!(
                                        "room participant count must be a non-negative integer, got {participant_count:?}"
                                    )
                                },
                            )?;
                            let maximum_drift =
                                Self::parse_optional_f64(maximum_drift, "maximum room drift")?;
                            let buffering_participants =
                                Self::parse_optional_text(buffering_participants)
                                    .map(|names| {
                                        names
                                            .split(',')
                                            .map(str::trim)
                                            .filter(|name| !name.is_empty())
                                            .map(str::to_owned)
                                            .collect()
                                    })
                                    .unwrap_or_default();
                            (participant_count, maximum_drift, buffering_participants)
                        }
                        _ => {
                            return Err(
                                "apply-main-window-room-intent accepts either five arguments or five plus participant count, maximum drift, and comma-separated buffering names"
                                    .to_owned(),
                            );
                        }
                    };
                Self::ApplyMainWindowRoomPlaybackIntent(MainWindowRoomPlaybackIntent {
                    position_seconds,
                    paused,
                    set_by,
                    authority,
                    start_gate,
                    participant_count,
                    maximum_observed_drift_seconds,
                    buffering_participants,
                })
            }
            "apply-main-window-participant-status" => {
                let username = fields
                    .next()
                    .ok_or_else(|| {
                        "apply-main-window-participant-status requires a username".to_owned()
                    })?
                    .to_owned();
                let mode = fields.next().ok_or_else(|| {
                    "apply-main-window-participant-status requires a status mode".to_owned()
                })?;
                let (status, start_barrier_status) = match mode {
                    "unavailable" | "legacy" | "waiting" => {
                        let status = match mode {
                            "unavailable" => MainWindowParticipantStatusPresentation::Unavailable,
                            "legacy" => MainWindowParticipantStatusPresentation::LegacyClient,
                            "waiting" => {
                                MainWindowParticipantStatusPresentation::WaitingForFirstReport
                            }
                            _ => unreachable!(),
                        };
                        let start_barrier_status =
                            Self::parse_optional_text(fields.next().ok_or_else(|| {
                                "apply-main-window-participant-status requires start barrier or 'none'"
                                    .to_owned()
                            })?);
                        (status, start_barrier_status)
                    }
                    "report" => {
                        let player = Self::parse_member_player_availability(
                            fields.next().ok_or_else(|| {
                                "participant status report requires player availability".to_owned()
                            })?,
                        )?;
                        let phase =
                            Self::parse_member_playback_phase(fields.next().ok_or_else(|| {
                                "participant status report requires playback phase".to_owned()
                            })?)?;
                        let position_seconds = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires position or 'none'".to_owned()
                            })?,
                            "member position",
                        )?;
                        let logical_paused =
                            Self::parse_optional_bool(fields.next().ok_or_else(|| {
                                "participant status report requires logical pause or 'none'"
                                    .to_owned()
                            })?)?;
                        let playback_rate = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires playback rate or 'none'"
                                    .to_owned()
                            })?,
                            "member playback rate",
                        )?;
                        let buffered_ahead_seconds = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires buffered ahead or 'none'"
                                    .to_owned()
                            })?,
                            "member buffered ahead",
                        )?;
                        let cache_refill_percent = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires cache refill or 'none'"
                                    .to_owned()
                            })?,
                            "member cache refill",
                        )?;
                        let room_offset_seconds = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires room offset or 'none'"
                                    .to_owned()
                            })?,
                            "member room offset",
                        )?;
                        let media_generation = Self::parse_optional_u64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires media generation or 'none'"
                                    .to_owned()
                            })?,
                            "member media generation",
                        )?;
                        let state_revision = Self::parse_optional_u64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires state revision or 'none'"
                                    .to_owned()
                            })?,
                            "member state revision",
                        )?;
                        let correlation =
                            Self::parse_participant_status_correlation(fields.next().ok_or_else(
                                || "participant status report requires correlation".to_owned(),
                            )?)?;
                        let sample_age_seconds = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires sample age or 'none'".to_owned()
                            })?,
                            "member sample age",
                        )?;
                        let position_sample_age_seconds = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires position sample age or 'none'"
                                    .to_owned()
                            })?,
                            "member position sample age",
                        )?;
                        let report_age_seconds = Self::parse_optional_f64(
                            fields.next().ok_or_else(|| {
                                "participant status report requires report age or 'none'".to_owned()
                            })?,
                            "member report age",
                        )?;
                        let timeline_mismatch =
                            Self::parse_bool(fields.next().ok_or_else(|| {
                                "participant status report requires timeline_mismatch".to_owned()
                            })?)?;
                        let start_barrier_status =
                            Self::parse_optional_text(fields.next().ok_or_else(|| {
                                "participant status report requires start barrier or 'none'"
                                    .to_owned()
                            })?);
                        let freshness = match report_age_seconds {
                            None => MainWindowParticipantStatusFreshness::Unknown,
                            Some(age) if age <= 3.0 => MainWindowParticipantStatusFreshness::Fresh,
                            Some(age) if age <= 10.0 => {
                                MainWindowParticipantStatusFreshness::Delayed
                            }
                            Some(_) => MainWindowParticipantStatusFreshness::Stale,
                        };
                        let availability = match freshness {
                            MainWindowParticipantStatusFreshness::Unknown => {
                                ParticipantStatusAvailability::AwaitingReport
                            }
                            MainWindowParticipantStatusFreshness::Fresh => {
                                ParticipantStatusAvailability::Fresh
                            }
                            MainWindowParticipantStatusFreshness::Delayed => {
                                ParticipantStatusAvailability::Delayed
                            }
                            MainWindowParticipantStatusFreshness::Stale => {
                                ParticipantStatusAvailability::Stale
                            }
                            _ => ParticipantStatusAvailability::Unavailable,
                        };
                        let mut status = ParticipantStatusView::new(availability);
                        status.correlation = correlation;
                        status.playback_scope = media_generation.map(|media_generation| {
                            let mut scope = ParticipantPlaybackScope::new(media_generation);
                            scope.state_revision = state_revision;
                            scope
                        });
                        status.player_connection = Some(player);
                        status.phase = Some(phase);
                        status.timeline_kind = Some(ParticipantTimelineKind::Unknown);
                        status.position_seconds = position_seconds;
                        status.logical_paused = logical_paused;
                        status.playback_rate = playback_rate;
                        status.paused_for_cache =
                            (phase == ParticipantPlaybackPhase::Rebuffering).then_some(true);
                        status.buffered_ahead_seconds = buffered_ahead_seconds;
                        status.cache_percent = cache_refill_percent;
                        status.sample_age_ms =
                            sample_age_seconds.map(|age| (age.max(0.0) * 1_000.0).round() as u64);
                        status.position_sample_age_ms = position_sample_age_seconds
                            .map(|age| (age.max(0.0) * 1_000.0).round() as u64);
                        status.report_age_ms =
                            report_age_seconds.map(|age| (age.max(0.0) * 1_000.0).round() as u64);
                        status.room_offset_seconds = room_offset_seconds;
                        (
                            MainWindowParticipantStatusPresentation::Report(
                                MainWindowParticipantStatusReport::from_client_view(
                                    ClientParticipantStatusView::from_wire(status),
                                    timeline_mismatch,
                                ),
                            ),
                            start_barrier_status,
                        )
                    }
                    _ => return Err(format!("unsupported participant status mode {mode:?}")),
                };
                if fields.next().is_some() {
                    return Err(
                        "apply-main-window-participant-status received too many arguments"
                            .to_owned(),
                    );
                }
                Self::ApplyMainWindowParticipantStatus {
                    username,
                    status,
                    start_barrier_status,
                }
            }
            "apply-player-setup-runtime" => {
                let kind =
                    Self::parse_player_setup_issue_kind(fields.next().ok_or_else(|| {
                        "apply-player-setup-runtime requires an issue kind".to_owned()
                    })?)?;
                let message = Self::decode_text_token(fields.next().ok_or_else(|| {
                    "apply-player-setup-runtime requires an issue message".to_owned()
                })?);
                if fields.next().is_some() {
                    return Err(
                        "apply-player-setup-runtime accepts exactly two arguments".to_owned()
                    );
                }
                Self::ApplyPlayerSetupRuntimeSnapshot(GuiPlayerSetupRuntimeSnapshot {
                    issue: Some(GuiPlayerSetupIssue {
                        retry_available: kind != GuiPlayerSetupIssueKind::NotConfigured,
                        kind,
                        message,
                    }),
                })
            }
            "apply-seek-preparation-runtime" => {
                let phase =
                    Self::parse_seek_preparation_phase(fields.next().ok_or_else(|| {
                        "apply-seek-preparation-runtime requires a phase".to_owned()
                    })?)?;
                let frozen_target_seconds = Self::parse_f64(
                    fields.next().ok_or_else(|| {
                        "apply-seek-preparation-runtime requires a frozen target".to_owned()
                    })?,
                    "frozen target",
                )?;
                let cache_refill_percent = Self::parse_optional_f64(
                    fields.next().ok_or_else(|| {
                        "apply-seek-preparation-runtime requires cache refill percent or 'none'"
                            .to_owned()
                    })?,
                    "cache refill percent",
                )?;
                let buffered_ahead_seconds = Self::parse_optional_f64(
                    fields.next().ok_or_else(|| {
                        "apply-seek-preparation-runtime requires buffered-ahead seconds or 'none'"
                            .to_owned()
                    })?,
                    "buffered-ahead seconds",
                )?;
                let nearest_safe_buffered_position_seconds = Self::parse_optional_f64(
                        fields.next().ok_or_else(|| {
                            "apply-seek-preparation-runtime requires nearest buffered position or 'none'"
                                .to_owned()
                        })?,
                        "nearest buffered position",
                    )?;
                let can_keep_waiting = Self::parse_bool(fields.next().ok_or_else(|| {
                    "apply-seek-preparation-runtime requires can_keep_waiting".to_owned()
                })?)?;
                let can_cancel_and_remain = Self::parse_bool(fields.next().ok_or_else(|| {
                    "apply-seek-preparation-runtime requires can_cancel_and_remain".to_owned()
                })?)?;
                let can_join_nearest_buffered =
                    Self::parse_bool(fields.next().ok_or_else(|| {
                        "apply-seek-preparation-runtime requires can_join_nearest_buffered"
                            .to_owned()
                    })?)?;
                if fields.next().is_some() {
                    return Err(
                        "apply-seek-preparation-runtime accepts exactly eight arguments".to_owned(),
                    );
                }
                Self::ApplySeekPreparationRuntimeSnapshot(GuiSeekPreparationRuntimeSnapshot {
                    preparation: Some(GuiSeekPreparationState {
                        phase,
                        frozen_target_seconds,
                        cache_refill_percent,
                        buffered_ahead_seconds,
                        nearest_safe_buffered_position_seconds,
                        can_keep_waiting,
                        can_cancel_and_remain,
                        can_join_nearest_buffered,
                    }),
                    degraded_reason: None,
                })
            }
            "apply-seek-preparation-degraded-runtime" => {
                let degraded_reason =
                    Self::parse_seek_preparation_degraded_reason(fields.next().ok_or_else(
                        || "apply-seek-preparation-degraded-runtime requires a reason".to_owned(),
                    )?)?;
                if fields.next().is_some() {
                    return Err(
                        "apply-seek-preparation-degraded-runtime accepts exactly one argument"
                            .to_owned(),
                    );
                }
                Self::ApplySeekPreparationRuntimeSnapshot(GuiSeekPreparationRuntimeSnapshot {
                    preparation: None,
                    degraded_reason: Some(degraded_reason),
                })
            }
            "clear-seek-preparation-runtime" => {
                if fields.next().is_some() {
                    return Err("clear-seek-preparation-runtime accepts no arguments".to_owned());
                }
                Self::ApplySeekPreparationRuntimeSnapshot(
                    GuiSeekPreparationRuntimeSnapshot::default(),
                )
            }
            "apply-main-window-playlist-selection" => {
                let index = Self::parse_optional_index(fields.next().ok_or_else(|| {
                    "apply-main-window-playlist-selection requires an index or 'none'".to_owned()
                })?)?;
                if fields.next().is_some() {
                    return Err(
                        "apply-main-window-playlist-selection accepts exactly one argument"
                            .to_owned(),
                    );
                }
                Self::ApplyMainWindowPlaylistSelection(index)
            }
            "push-chat-message" => {
                let sender = fields
                    .next()
                    .ok_or_else(|| "push-chat-message requires a sender".to_owned())?;
                let message = fields
                    .next()
                    .ok_or_else(|| "push-chat-message requires a message".to_owned())?;
                if fields.next().is_some() {
                    return Err("push-chat-message accepts exactly two arguments".to_owned());
                }
                Self::PushChatMessage {
                    sender: sender.to_owned(),
                    message: message.to_owned(),
                }
            }
            "open-media-files" => {
                let entries = Self::split_list_token(
                    fields
                        .next()
                        .ok_or_else(|| "open-media-files requires one or more paths".to_owned())?,
                )
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
                if fields.next().is_some() {
                    return Err("open-media-files accepts exactly one argument".to_owned());
                }
                if entries.is_empty() {
                    return Err("open-media-files requires at least one path".to_owned());
                }
                Self::OpenMediaFiles(entries)
            }
            "drop-media-files" => {
                let target = Self::parse_drop_target(
                    fields
                        .next()
                        .ok_or_else(|| "drop-media-files requires a target".to_owned())?,
                )?;
                let entries = Self::split_list_token(
                    fields
                        .next()
                        .ok_or_else(|| "drop-media-files requires one or more paths".to_owned())?,
                )
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
                if fields.next().is_some() {
                    return Err("drop-media-files accepts exactly two arguments".to_owned());
                }
                if entries.is_empty() {
                    return Err("drop-media-files requires at least one path".to_owned());
                }
                Self::DropMediaFiles {
                    target,
                    paths: entries,
                }
            }
            "add-media-search-directory" => {
                let path = fields
                    .next()
                    .ok_or_else(|| "add-media-search-directory requires a path".to_owned())?;
                if fields.next().is_some() {
                    return Err(
                        "add-media-search-directory accepts exactly one argument".to_owned()
                    );
                }
                Self::AddMediaSearchDirectory(path.to_owned())
            }
            "activate" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "activate requires a widget id".to_owned())?;
                if fields.next().is_some() {
                    return Err("activate accepts exactly one argument".to_owned());
                }
                Self::activate(widget_id)
            }
            "enter-text" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "enter-text requires a widget id".to_owned())?;
                let submit = Self::parse_bool(
                    fields
                        .next()
                        .ok_or_else(|| "enter-text requires a submit flag".to_owned())?,
                )?;
                let value = fields
                    .next()
                    .ok_or_else(|| "enter-text requires a value".to_owned())?;
                if fields.next().is_some() {
                    return Err("enter-text accepts exactly three arguments".to_owned());
                }
                Self::enter_text(widget_id, &Self::decode_text_token(value), submit)
            }
            "assert-label" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "assert-label requires a widget id".to_owned())?;
                let label = fields
                    .next()
                    .ok_or_else(|| "assert-label requires a label".to_owned())?;
                if fields.next().is_some() {
                    return Err("assert-label accepts exactly two arguments".to_owned());
                }
                Self::assert_widget_label(widget_id, label)
            }
            "assert-value" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "assert-value requires a widget id".to_owned())?;
                let value = fields
                    .next()
                    .ok_or_else(|| "assert-value requires a value token".to_owned())?;
                if fields.next().is_some() {
                    return Err("assert-value accepts exactly two arguments".to_owned());
                }
                let parsed_value = Self::parse_optional_value(value);
                Self::assert_widget_value(widget_id, parsed_value.as_deref())
            }
            "assert-selected" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "assert-selected requires a widget id".to_owned())?;
                let selected = Self::parse_bool(
                    fields
                        .next()
                        .ok_or_else(|| "assert-selected requires a boolean".to_owned())?,
                )?;
                if fields.next().is_some() {
                    return Err("assert-selected accepts exactly two arguments".to_owned());
                }
                Self::assert_widget_selected(widget_id, selected)
            }
            "assert-enabled" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "assert-enabled requires a widget id".to_owned())?;
                let enabled = Self::parse_bool(
                    fields
                        .next()
                        .ok_or_else(|| "assert-enabled requires a boolean".to_owned())?,
                )?;
                if fields.next().is_some() {
                    return Err("assert-enabled accepts exactly two arguments".to_owned());
                }
                Self::assert_widget_enabled(widget_id, enabled)
            }
            "assert-tone" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "assert-tone requires a widget id".to_owned())?;
                let tone = Self::parse_status_tone(
                    fields
                        .next()
                        .ok_or_else(|| "assert-tone requires a tone".to_owned())?,
                )?;
                if fields.next().is_some() {
                    return Err("assert-tone accepts exactly two arguments".to_owned());
                }
                Self::assert_widget_tone(widget_id, tone)
            }
            "assert-tooltip-contains" => {
                let widget_id = fields
                    .next()
                    .ok_or_else(|| "assert-tooltip-contains requires a widget id".to_owned())?;
                let text = fields
                    .next()
                    .ok_or_else(|| "assert-tooltip-contains requires expected text".to_owned())?;
                if fields.next().is_some() {
                    return Err("assert-tooltip-contains accepts exactly two arguments".to_owned());
                }
                Self::assert_widget_tooltip_contains(widget_id, text)
            }
            "assert-pending" => {
                let pending = Self::parse_pending(
                    fields
                        .next()
                        .ok_or_else(|| "assert-pending requires a pending label".to_owned())?,
                )?;
                if fields.next().is_some() {
                    return Err("assert-pending accepts exactly one argument".to_owned());
                }
                Self::assert_pending(pending)
            }
            "complete-pending" => {
                if fields.next().is_some() {
                    return Err("complete-pending does not accept arguments".to_owned());
                }
                Self::CompletePending
            }
            "cancel-pending" => {
                if fields.next().is_some() {
                    return Err("cancel-pending does not accept arguments".to_owned());
                }
                Self::CancelPending
            }
            "complete-pending-runtime" => {
                if fields.next().is_some() {
                    return Err("complete-pending-runtime does not accept arguments".to_owned());
                }
                Self::CompletePendingRuntime
            }
            "close-modal" => {
                if fields.next().is_some() {
                    return Err("close-modal does not accept arguments".to_owned());
                }
                Self::CloseModal
            }
            "clear-notifications" => {
                if fields.next().is_some() {
                    return Err("clear-notifications does not accept arguments".to_owned());
                }
                Self::ClearNotifications
            }
            _ => return Err(format!("unknown semantic script command {command:?}")),
        };

        Ok(Some(step))
    }

    pub(in crate::app) fn parse_script(script: &str) -> Result<Vec<Self>, String> {
        script
            .lines()
            .enumerate()
            .filter_map(|(line_index, line)| match Self::from_script_line(line) {
                Ok(Some(step)) => Some(Ok(step)),
                Ok(None) => None,
                Err(error) => Some(Err(format!(
                    "semantic script line {} failed: {error}",
                    line_index + 1
                ))),
            })
            .collect()
    }
}
