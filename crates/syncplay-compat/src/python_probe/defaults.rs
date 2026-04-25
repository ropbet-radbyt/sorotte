use super::*;

pub fn default_rust_client_hello_for_interop() -> HelloPayload {
    HelloPayload::new("interop-client", "interop-room", "1.2.255")
        .with_realversion("syncplay-rs-dev")
        .with_features(json!({ "featureList": true }))
}

#[cfg(test)]
pub(crate) fn default_rust_client_hello_for_legacy_live_tls() -> HelloPayload {
    // Legacy live-server handshake paths parse version-like strings as dotted integers.
    HelloPayload::new("interop-client", "interop-room", "1.2.255")
        .with_realversion("1.2.255")
        .with_features(json!({ "featureList": true }))
}
