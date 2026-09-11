use super::*;

use serde_json::Value;
use sorotte_client_app::app_boundary::state::{
    AutoplayThresholdOverride, StoredClientSettings, stored_client_settings_runtime_snapshot,
};
use sorotte_client_core::{
    DesyncCorrectionConfig, ReadinessAutoplayConfig, SYNCPLAY_COMPAT_VERSION,
    SYNCPLAY_WIRE_VERSION, SessionBehaviorConfig, UnpauseActionMode,
};
use sorotte_protocol::{ProtocolMessage, SOROTTE_READINESS_RECONNECT_TOKEN, decode_message_line};

mod readiness_and_defaults;
mod settings_sync_and_incremental;
mod startup_and_identity;
