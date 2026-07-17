mod adapter;
mod constants;
mod ipc;
mod legacy_ui;
mod live_probe;
mod players;
#[cfg(feature = "test-support")]
mod test_support;

/// Maximum absolute position error accepted when an mpv seek is acknowledged.
///
/// mpv can report a nearby decoded timestamp rather than the exact requested
/// floating-point value. Completion still additionally requires a matching
/// generation and an observation that `seeking` is false.
pub const MPV_SEEK_COMPLETION_TOLERANCE_SECONDS: f64 = 0.5;

pub use adapter::MpvAdapter;
pub use ipc::MpvIpcConnectionEvent;
pub use legacy_ui::{LegacySyncplayOsdKind, LegacySyncplayUiSettings};
pub use players::{ConnectedMpvPlayer, SimulatedPlayer};

#[cfg(test)]
mod tests;
