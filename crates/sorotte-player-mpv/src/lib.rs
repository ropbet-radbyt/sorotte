mod adapter;
mod constants;
mod ipc;
mod legacy_ui;
mod players;

pub use adapter::MpvAdapter;
pub use ipc::MpvIpcConnectionEvent;
pub use legacy_ui::{LegacySyncplayOsdKind, LegacySyncplayUiSettings};
pub use players::{ConnectedMpvPlayer, SimulatedPlayer};

#[cfg(test)]
mod tests;
