use sorotte_core::SyncDomain;

mod network_harness;
mod playback_harness;

pub use network_harness::{
    BurstStall, FaultInjectingHttpServer, HttpMediaFixture, HttpRequestRecord, NetworkFaultProfile,
    dash_static_manifest, hls_sliding_window_manifest, hls_vod_manifest,
};
pub use playback_harness::{MultiClientPlaybackHarness, RecordedPlaybackCommand};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VirtualClock {
    now_ms: u64,
}

impl VirtualClock {
    pub fn now_ms(self) -> u64 {
        self.now_ms
    }

    pub fn advance_ms(&mut self, delta_ms: u64) {
        self.now_ms = self.now_ms.saturating_add(delta_ms);
    }
}

pub fn run_ready_scenario() -> bool {
    let mut domain = SyncDomain::default();
    domain.join_room("alice", "room1");
    domain.join_room("bob", "room1");

    if domain.set_ready("alice", "room1", true).is_err() {
        return false;
    }
    if domain.set_ready("bob", "room1", true).is_err() {
        return false;
    }

    domain
        .users_in_room("room1")
        .map(|users| users.iter().all(|user| user.ready == Some(true)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::{VirtualClock, run_ready_scenario};

    #[test]
    fn virtual_clock_is_deterministic() {
        let mut clock = VirtualClock::default();
        clock.advance_ms(10);
        clock.advance_ms(25);
        assert_eq!(clock.now_ms(), 35);
    }

    #[test]
    fn ready_scenario_smoke_test() {
        assert!(run_ready_scenario());
    }
}
