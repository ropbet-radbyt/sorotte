mod adapter;
mod bridge;
mod bridge_resource;
mod constants;
mod ipc;
mod legacy_ui;
pub mod lifecycle;
mod players;
#[cfg(feature = "test-support")]
mod test_support;
pub mod transcript;

#[cfg(feature = "fuzz-support")]
#[doc(hidden)]
pub mod fuzz_support {
    pub use crate::ipc::{
        FuzzMpvIpcOutcome, FuzzMpvIpcRun, FuzzMpvIpcScriptEnd, run_in_memory_mpv_ipc_fuzz_case,
    };
}

use sorotte_player_api::PlayerError;

/// Oldest mpv release supported by Sorotte's JSON IPC adapter.
pub const MINIMUM_SUPPORTED_MPV_VERSION: &str = "0.41.0";
pub(crate) const UNSUPPORTED_MPV_VERSION_ERROR_PREFIX: &str = "Sorotte requires mpv ";

/// Returns whether an adapter error specifically rejects an unsupported or unverifiable mpv
/// version.
pub fn is_unsupported_mpv_version_error(error: &PlayerError) -> bool {
    matches!(
        error,
        PlayerError::OperationFailed(message)
            if message.starts_with(UNSUPPORTED_MPV_VERSION_ERROR_PREFIX)
    )
}

/// Maximum absolute position error accepted when an mpv seek is acknowledged.
///
/// mpv can report a nearby decoded timestamp rather than the exact requested
/// floating-point value. Completion still additionally requires a matching
/// generation and an observation that `seeking` is false.
pub const MPV_SEEK_COMPLETION_TOLERANCE_SECONDS: f64 = 0.5;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use adapter::{
    LifecycleVerificationPlaylistEntry, LifecycleVerificationTrackedLoad,
    MpvLifecycleVerificationHarness,
};
pub use adapter::{
    MpvActiveNetworkMediaOptionsApplyOutcome, MpvAdapter, MpvNetworkMediaDiagnosticSnapshot,
    MpvNetworkMediaOptionsTransitionOutcome, MpvNetworkMediaPolicyApplicationState,
    MpvNetworkMediaPolicyOutcome, MpvNetworkMediaPolicyState, MpvNetworkOptionApplyResult,
    MpvNetworkOptionApplyStatus, MpvNetworkOptionsHookHealth,
    MpvNetworkOptionsHookHealthTransition, MpvNetworkOptionsRuntimeHealthSnapshot,
};
pub use bridge::{SorotteBridgeFailure, SorotteBridgeFailureKind, SorotteBridgeHealth};
pub use bridge_resource::{
    materialize_bundled_sorotte_bridge, materialize_bundled_sorotte_bridge_in,
    materialize_bundled_sorotte_network_options_hook,
    materialize_bundled_sorotte_network_options_hook_in,
};
pub use ipc::MpvIpcConnectionEvent;
pub use legacy_ui::{LegacySyncplayOsdKind, LegacySyncplayUiSettings};
pub use players::{ConnectedMpvPlayer, SimulatedPlayer};

#[cfg(test)]
mod tests;
