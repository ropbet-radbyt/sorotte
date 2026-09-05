//! The public verification harness drives real adapter/reducer ingress without external mpv.
use super::{Fixture, metrics};
use serde_json::{Value, json};
use sorotte_player_api::SnapshotField;
use sorotte_player_mpv::{LifecycleVerificationPlaylistEntry, MpvLifecycleVerificationHarness};

fn acknowledge(harness: &mut MpvLifecycleVerificationHarness) -> Result<(), String> {
    for _ in 0..256 {
        let Some(batch) = harness.take_event_batch() else {
            return Ok(());
        };
        harness
            .acknowledge(batch.acknowledgement_token)
            .map_err(|e| e.to_string())?;
    }
    Err("recovery acknowledgement did not drain".to_owned())
}

fn load(harness: &mut MpvLifecycleVerificationHarness, entry: i64, path: &str) {
    harness.apply_authoritative_snapshot(
        [LifecycleVerificationPlaylistEntry::new(
            entry,
            Some(path.to_owned()),
            true,
        )],
        Some(path.to_owned()),
    );
    harness.ingest_decoded_mpv_json(json!({"event":"start-file","playlist_entry_id":entry}));
    harness.ingest_decoded_mpv_json(json!({"event":"property-change","name":"path","data":path}));
    harness.ingest_decoded_mpv_json(json!({"event":"file-loaded"}));
}

pub fn run(fixture: Fixture) -> Result<Value, String> {
    let mut harness = MpvLifecycleVerificationHarness::new();
    let mut checkpoints = Vec::new();
    let mut maximum_attempts = 0;
    let ((), cost) = metrics::measure(|| {
        for cycle in 0..fixture.churn_cycles {
            let path = format!("https://fixture.invalid/media-{cycle:06}.mkv");
            let initial_entry = cycle as i64 * 2 + 1;
            let tracked = harness.accept_tracked_load(&path, []);
            load(&mut harness, initial_entry, &path);
            acknowledge(&mut harness)?;
            harness.accept_same_generation_recovery(
                tracked.media_generation,
                &path,
                [initial_entry],
            );
            harness.ingest_decoded_mpv_json(
                json!({"event":"end-file","playlist_entry_id":initial_entry,"reason":"stop"}),
            );
            load(&mut harness, initial_entry + 1, &path);
            harness.ingest_decoded_mpv_json(
                json!({"event":"end-file","playlist_entry_id":initial_entry+1,"reason":"stop"}),
            );
            harness.apply_authoritative_snapshot([], None);
            acknowledge(&mut harness)?;
            harness.advance_clock(1000);
            acknowledge(&mut harness)?;
            let projection = harness.projection();
            maximum_attempts = maximum_attempts.max(projection.attempts.len());
            let SnapshotField::Known(pending) = projection.pending_event_count else {
                return Err("pending event count unavailable".to_owned());
            };
            let SnapshotField::Known(outcomes) = projection.retained_semantic_outcome_count else {
                return Err("retained outcome count unavailable".to_owned());
            };
            if projection.attempts.len() > 2 || pending != 0 || outcomes != 0 {
                return Err(format!(
                    "recovery retained ownership after acknowledgement: {} attempts, {pending} events, {outcomes} outcomes",
                    projection.attempts.len()
                ));
            }
            if (cycle + 1) % (fixture.churn_cycles / 8).max(1) == 0
                || cycle + 1 == fixture.churn_cycles
            {
                checkpoints.push(json!({"completed_cycles":cycle+1,"retained_attempts":projection.attempts.len(),"pending_events":pending,"retained_outcomes":outcomes}));
                harness.replace_attachment();
                acknowledge(&mut harness)?;
            }
        }
        Ok(())
    })?;
    Ok(
        json!({"allocation":cost,"cycles":fixture.churn_cycles,"maximum_retained_attempts":maximum_attempts,
        "checkpoints":checkpoints,"external_mpv":false,"transport":"deterministic adapter verification ingress"}),
    )
}
