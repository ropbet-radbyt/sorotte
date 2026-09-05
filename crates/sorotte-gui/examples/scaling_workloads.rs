//! Deterministic fixtures over production APIs. Invoke through scripts/scaling_workloads.py.
#[path = "scaling/media.rs"]
mod media;
#[path = "scaling/metrics.rs"]
mod metrics;
#[path = "scaling/recovery.rs"]
mod recovery;
#[path = "scaling/server.rs"]
mod server;

use serde_json::{Value, json};
use std::path::PathBuf;

#[global_allocator]
static ALLOCATOR: metrics::CountingAllocator = metrics::CountingAllocator;

#[derive(Clone, Copy, serde::Serialize)]
struct Fixture {
    name: &'static str,
    roster: usize,
    empty_rooms: usize,
    metadata_bytes: usize,
    playlist_items: usize,
    server_playlist_items: usize,
    inventory: usize,
    anchors_per_file: usize,
    gui_pumps: usize,
    churn_cycles: usize,
}

impl Fixture {
    fn named(name: &str) -> Result<Self, String> {
        match name {
            "normal" => Ok(Self {
                name: "normal",
                roster: 4,
                empty_rooms: 8,
                metadata_bytes: 128,
                playlist_items: 16,
                server_playlist_items: 16,
                inventory: 64,
                anchors_per_file: 32,
                gui_pumps: 16,
                churn_cycles: 32,
            }),
            "large" => Ok(Self {
                name: "large",
                roster: 64,
                empty_rooms: 512,
                metadata_bytes: 1024,
                playlist_items: 2048,
                server_playlist_items: 250,
                inventory: 1024,
                anchors_per_file: 32,
                gui_pumps: 32,
                churn_cycles: 256,
            }),
            _ => Err("case must be normal or large".to_owned()),
        }
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let base = std::env::temp_dir();
        for attempt in 0..128 {
            let path = base.join(format!("sorotte-scaling-{}-{attempt}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("scaling fixture directory unavailable".to_owned())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run() -> Result<Value, String> {
    let mut arguments = std::env::args().skip(1);
    let fixture = Fixture::named(&arguments.next().unwrap_or_else(|| "normal".to_owned()))?;
    let mut extra_clone = false;
    let mut fixture = fixture;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--inject-extra-roster-clone" => extra_clone = true,
            "--churn-cycles" => {
                fixture.churn_cycles = arguments
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| (1..=100_000).contains(value))
                    .ok_or("churn cycles must be in 1..=100000")?;
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    let scratch = Scratch::new()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let server_model = server::model(fixture, extra_clone)?;
    let network = runtime.block_on(server::network(fixture))?;
    let media = media::run(fixture, &scratch.0)?;
    let recovery = recovery::run(fixture)?;
    let users = (0..fixture.roster)
        .map(|i| format!("user-{i:04},true,false,false"))
        .collect::<Vec<_>>()
        .join("|");
    let playlist = (0..fixture.playlist_items)
        .map(|i| format!("episode-{i:04}.mkv"))
        .collect::<Vec<_>>()
        .join("|");
    let script = format!(
        "apply-main-window-runtime\troom\ttrue\tfalse\tfalse\ttrue\ttrue\tfalse\ttrue\t{users}\t{playlist}\tsystem>connected"
    );
    let (gui, gui_allocations) = metrics::measure(|| {
        sorotte_gui::semantic_smoke::measure_projection(&script, fixture.gui_pumps)
    })?;
    Ok(
        json!({ "schema": "sorotte-scaling-sample-v1", "fixture_version": 2, "fixture": fixture,
        "extra_roster_clone": extra_clone, "server": server_model, "network": network, "media": media, "recovery": recovery,
        "gui": { "projection": gui, "allocations_including_setup": gui_allocations, "native_rendering": false },
        "correctness": "passed" }),
    )
}

fn main() {
    match run() {
        Ok(report) => println!(
            "{}",
            serde_json::to_string(&report).expect("finite benchmark JSON")
        ),
        Err(error) => {
            eprintln!("scaling workload failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_full_roster_clone_increases_real_allocations_and_keeps_dispatch_identical() {
        let mut fixture = Fixture::named("normal").unwrap();
        fixture.churn_cycles = 1;
        let control = server::model(fixture, false).unwrap();
        let injected = server::model(fixture, true).unwrap();
        assert_eq!(control["list"]["dispatch"], injected["list"]["dispatch"]);
        for metric in ["allocation_calls", "allocated_bytes"] {
            assert!(
                injected["list"]["allocation"][metric].as_u64().unwrap()
                    > control["list"]["allocation"][metric].as_u64().unwrap()
            );
        }
        let mut large = Fixture::named("large").unwrap();
        large.churn_cycles = 1;
        server::model(large, false)
            .expect("large playlist fixture must be accepted by every real recipient");
    }
}
