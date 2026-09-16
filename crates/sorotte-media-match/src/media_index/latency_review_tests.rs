//! Opt-in experiment for the lifecycle latency review; never part of the regular test run.
use super::*;
use std::time::Instant;

#[test]
#[ignore = "writes synthetic indexes under SOROTTE_LATENCY_OUTPUT and measures real SQLite transactions"]
fn latency_review_index_transaction_scaling() {
    let output =
        PathBuf::from(std::env::var_os("SOROTTE_LATENCY_OUTPUT").expect("explicit output"));
    fs::create_dir_all(&output).unwrap();
    let mut rows = Vec::new();
    for mib in [0_usize, 8, 32, 98] {
        let live = output.join(format!("index-{mib}"));
        assert!(!live.exists(), "use a fresh evidence directory");
        let connection = open_media_match_v3_index(&live).unwrap();
        connection
            .execute_batch("CREATE TABLE latency_review_padding(data BLOB); BEGIN")
            .unwrap();
        for _ in 0..mib {
            connection
                .execute(
                    "INSERT INTO latency_review_padding VALUES (zeroblob(1048576))",
                    [],
                )
                .unwrap();
        }
        connection
            .execute_batch("COMMIT; PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        drop(connection);
        for trial in 0..3 {
            let start = Instant::now();
            let transaction = MediaIndexBuildTransaction::begin(
                &live,
                output.join(format!("stage-{mib}-{trial}")),
            )
            .unwrap();
            let begin_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            transaction.commit().unwrap();
            let commit_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let session = MediaIndexService::new(&live).open().unwrap();
            let open_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            for _ in 0..100 {
                session
                    .load_record(
                        "latency-review-missing.mkv",
                        &crate::MediaExtractionSettings::default(),
                        0,
                        0,
                    )
                    .unwrap();
            }
            let retained_lookup_mean_ms = start.elapsed().as_secs_f64() * 10.0;
            drop(session);
            let row = serde_json::json!({
                "padding_mib":mib, "trial":trial, "database_bytes":fs::metadata(MediaIndexService::new(&live).index_path()).unwrap().len(),
                "begin_ms":begin_ms, "commit_ms":commit_ms, "activated_open_ms":open_ms,
                "retained_lookup_mean_ms":retained_lookup_mean_ms,
                "backup_pages_per_step":media_index_backup_pages_per_step()
            });
            println!("LATENCY {row}");
            rows.push(row);
        }
    }
    fs::write(
        output.join("index-timings.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
