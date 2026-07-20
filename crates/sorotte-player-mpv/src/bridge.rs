use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SorotteBridgeHealth {
    Disabled,
    Ready,
    /// The bridge was ready, but is temporarily reacquiring ownership or rediscovering the
    /// stable Lua target. Core mpv JSON IPC remains usable while player chat is gated.
    Recovering,
    Degraded(SorotteBridgeFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorotteBridgeFailureKind {
    ResourceMaterialization,
    Discovery,
    ScriptLoad,
    LeaseBusy,
    SettingsRejected,
    AcknowledgementTimeout,
    IpcCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorotteBridgeFailure {
    pub kind: SorotteBridgeFailureKind,
    pub reason: String,
}

impl SorotteBridgeFailure {
    pub(crate) fn new(kind: SorotteBridgeFailureKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            reason: reason.into(),
        }
    }

    pub fn retryable_in_place(&self) -> bool {
        true
    }
}

impl fmt::Display for SorotteBridgeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.reason)
    }
}

impl std::error::Error for SorotteBridgeFailure {}
