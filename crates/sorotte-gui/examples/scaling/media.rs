use super::{Fixture, metrics};
use serde_json::{Value, json};
use sorotte_media_match::{
    AudioAnchor, MEDIA_MATCH_ALGORITHM_VERSION, MediaExtractionSettings, MediaFileIdentity,
    MediaFingerprintRecord, MediaIndexBuildTransaction, MediaIndexInventoryEntry,
    MediaIndexService,
};
use std::path::Path;

pub fn run(fixture: Fixture, scratch: &Path) -> Result<Value, String> {
    let service = MediaIndexService::new(scratch.join("media"));
    let session = service.open()?;
    let settings = MediaExtractionSettings::sampled_fast_audio_index_v3();
    let entries = (0..fixture.inventory)
        .map(|i| MediaIndexInventoryEntry::new(format!("/scaling/episode-{i:06}.mkv"), 1, 123456))
        .collect::<Vec<_>>();
    let paths = entries
        .iter()
        .map(|entry| entry.normalized_path.clone())
        .collect::<Vec<_>>();
    let ((), inventory_cost) = metrics::measure(|| {
        session.refresh_inventory(&entries, &paths, &["/scaling".to_owned()], || false)
    })?;
    let ((), fingerprint_cost) = metrics::measure(|| {
        for (i, entry) in entries.iter().enumerate() {
            session.save_record(
                &MediaFingerprintRecord {
                    identity: MediaFileIdentity {
                        normalized_path: entry.normalized_path.clone(),
                        modified_unix_millis: 1,
                        size_bytes: 123456,
                    },
                    algorithm_version: MEDIA_MATCH_ALGORITHM_VERSION,
                    extraction_settings: settings.clone(),
                    duration_seconds: Some(1440.0),
                    container_fingerprint: format!("fixture-{i:06}"),
                    // Four files per bucket family yield deterministic useful hits without a common
                    // bucket degenerating into a scan of the entire generated inventory.
                    audio_anchors: (0..fixture.anchors_per_file)
                        .map(|a| AudioAnchor {
                            bucket: 100 + (i as u32 / 4) * 64 + (a as u32 % 12),
                            t_ms: a as u32 * 1000 + (i as u32 % 4) * 25,
                            weight: 10,
                        })
                        .collect(),
                    audio_error: None,
                },
                None,
            )?;
        }
        Ok(())
    })?;
    let (warm, warm_cost) =
        metrics::measure(|| session.anchor_candidate_paths(&paths[0], &settings))?;
    let (warm_repeat, warm_repeat_cost) =
        metrics::measure(|| session.anchor_candidate_paths(&paths[0], &settings))?;
    if warm.0 != warm_repeat.0
        || warm.0.is_empty()
        || warm.0.len() > 4
        || warm.1.raw_hit_rows_processed > (fixture.anchors_per_file * 16) as i64
    {
        return Err(
            "generated index retrieval lost useful matches or exceeded its fixture work bound"
                .to_owned(),
        );
    }
    let summary = session.summary(&settings)?;
    drop(session);
    let (cold, cold_cost) =
        metrics::measure(|| service.open()?.anchor_candidate_paths(&paths[0], &settings))?;
    if warm.0 != cold.0 {
        return Err("warm/reopened index retrieval differs".to_owned());
    }
    let staging = scratch.join("cancelled-rebuild");
    let mut checks = 0;
    let ((), cancellation_cost) = metrics::measure(|| {
        let transaction = MediaIndexBuildTransaction::begin(service.root(), &staging)?;
        let staged_session = MediaIndexService::new(transaction.staging_root()).open()?;
        let changed = entries
            .iter()
            .map(|entry| MediaIndexInventoryEntry::new(&entry.normalized_path, 2, 123457))
            .collect::<Vec<_>>();
        let result =
            staged_session.refresh_inventory(&changed, &paths, &["/scaling".to_owned()], || {
                checks += 1;
                checks > fixture.inventory / 2
            });
        if result.is_ok() {
            return Err("rebuild ignored cancellation".to_owned());
        }
        drop(staged_session);
        drop(transaction);
        Ok(())
    })?;
    let live = service.open()?;
    if staging.exists()
        || live.inventory_paths()? != paths
        || live.summary(&settings)?.current_settings_fingerprint_count != fixture.inventory
        || live.load_record(&paths[0], &settings, 1, 123456)?.is_none()
    {
        return Err("cancelled rebuild changed live identity or retained staging".to_owned());
    }
    Ok(
        json!({"inventory":inventory_cost,"fingerprint_build":fingerprint_cost,
        "warm_initial":{"allocation":warm_cost,"stats":warm.1},"warm_repeat":{"allocation":warm_repeat_cost,"stats":warm_repeat.1},
        "cold_reopened":{"allocation":cold_cost,"stats":cold.1,"os_page_cache_evicted":false},
        "cancellation":cancellation_cost,"cancellation_checks":checks,"retained_staging_directories":0,
        "inventory_count":summary.inventory_count,"fingerprint_count":summary.current_settings_fingerprint_count,
        "database_bytes":summary.database_bytes,"audio_blob_bytes":summary.v3_audio_blob_bytes,"useful_candidates":warm.0.len()}),
    )
}
