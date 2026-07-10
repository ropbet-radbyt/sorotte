mod adapter;
mod constants;
mod ipc;
mod legacy_ui;

pub use adapter::MpvAdapter;
pub use ipc::MpvIpcConnectionEvent;
pub use legacy_ui::{LegacySyncplayOsdKind, LegacySyncplayUiSettings};

#[cfg(test)]
mod tests;
