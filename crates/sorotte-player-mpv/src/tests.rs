use super::{LegacySyncplayOsdKind, LegacySyncplayUiSettings, MpvAdapter, SimulatedPlayer};
use crate::ipc::{MpvJsonIpcTransport, read_line_from_stream};
use serde_json::{Value, json};
use sorotte_player_api::{
    LocalFileUpdate, PlayerAdapter, PlayerCommand, PlayerError, PlayerMediaLoadFailureKind,
    PlayerMediaLoadOutcome, PlayerPlaybackTelemetryUpdate,
};
use std::{
    collections::VecDeque,
    fs::File,
    io,
    io::Write,
    sync::{Arc, Mutex},
};

mod event_tests;
mod ipc_tests;
mod legacy_ui_tests;
#[cfg(windows)]
mod smoke_tests;
mod state_tests;

fn fake_transport_with_reads(lines: &[&str]) -> (FakeTransport, FakeTransportStateHandle) {
    let shared = Arc::new(Mutex::new(FakeTransportState {
        reads: lines
            .iter()
            .map(|line| {
                let mut owned = (*line).to_owned();
                owned.push('\n');
                owned
            })
            .collect(),
        writes: Vec::new(),
    }));
    (
        FakeTransport {
            shared: Arc::clone(&shared),
        },
        FakeTransportStateHandle { shared },
    )
}

#[derive(Debug)]
struct FakeTransport {
    shared: Arc<Mutex<FakeTransportState>>,
}

impl MpvJsonIpcTransport for FakeTransport {
    fn send_line_until(&mut self, line: &str, _deadline: std::time::Instant) -> io::Result<()> {
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .writes
            .push(line.to_owned());
        Ok(())
    }

    fn read_line_until(
        &mut self,
        line: &mut String,
        _deadline: std::time::Instant,
    ) -> io::Result<usize> {
        let mut guard = self
            .shared
            .lock()
            .expect("fake transport mutex should not be poisoned");
        let Some(next) = guard.reads.pop_front() else {
            line.clear();
            return Ok(0);
        };
        line.clear();
        line.push_str(&next);
        Ok(line.len())
    }
}

#[derive(Debug)]
struct FakeTransportState {
    reads: VecDeque<String>,
    writes: Vec<String>,
}

#[derive(Debug)]
struct FakeTransportStateHandle {
    shared: Arc<Mutex<FakeTransportState>>,
}

impl FakeTransportStateHandle {
    fn writes(&self) -> Vec<String> {
        self.shared
            .lock()
            .expect("fake transport mutex should not be poisoned")
            .writes
            .clone()
    }
}
