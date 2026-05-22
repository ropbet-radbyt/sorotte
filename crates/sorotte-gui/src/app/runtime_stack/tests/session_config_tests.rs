use super::*;

use sorotte_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettingsMvp,
    stored_client_settings_runtime_snapshot_legacy_compatible,
};
use sorotte_client_core::{
    DesyncCorrectionConfig, ReadinessAutoplayConfig, SYNCPLAY_COMPAT_VERSION_LEGACY,
    SYNCPLAY_WIRE_VERSION_LEGACY, SessionBehaviorConfig, UnpauseActionMode,
};
use sorotte_protocol::{ProtocolMessage, decode_message_line};

mod readiness_and_defaults;
mod settings_sync_and_incremental;
mod startup_and_identity;
