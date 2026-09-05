use super::*;
use std::{io::Cursor, net::TcpListener, thread, time::SystemTime};

struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("sorotte-ingress-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn serve(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 8192];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(&response);
    });
    (url, worker)
}

#[test]
fn remote_metadata_bounds_declared_chunked_and_missing_length_bodies() {
    for response in [
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            METADATA_BYTES + 1
        ),
        format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            METADATA_BYTES + 1,
            " ".repeat(METADATA_BYTES as usize + 1)
        ),
        format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{}",
            " ".repeat(METADATA_BYTES as usize + 1)
        ),
    ] {
        let (url, worker) = serve(response.into_bytes());
        let result = github_get_json::<serde_json::Value>(&github_http_client().unwrap(), &url);
        assert!(result.unwrap_err().contains("byte budget"));
        worker.join().unwrap();
    }
    let (url, worker) = serve(
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n"
            .to_vec(),
    );
    assert!(
        fetch_public_servers_from_url(&url, None)
            .unwrap_err()
            .contains("HTTP 503")
    );
    worker.join().unwrap();
}

#[test]
fn archive_quotas_reject_expanding_and_many_entry_inputs_before_output() {
    let root = TestRoot::new();
    let path = root.0.join("expanding.zip");
    fs::write(&path, zip_bytes(&[("data.bin", &vec![0; 32 * 1024])])).unwrap();
    let mut budget = ExtractionBudget::with_limits(10, 16 * 1024, 16 * 1024);
    let output = root.0.join("expanded");
    assert!(
        extract_zip_file_safe(&path, &output, &mut budget, &AtomicBool::new(false))
            .unwrap_err()
            .contains("byte budget")
    );
    assert!(!output.exists());
    let path = root.0.join("many.zip");
    fs::write(&path, zip_bytes(&[("a", b""), ("b", b""), ("c", b"")])).unwrap();
    let mut budget = ExtractionBudget::with_limits(2, 100, 100);
    assert!(
        extract_zip_file_safe(&path, &output, &mut budget, &AtomicBool::new(false))
            .unwrap_err()
            .contains("entry-count")
    );
    assert!(!output.exists());
}

#[test]
fn duplicate_normalized_zip_paths_are_rejected_before_any_file_is_written() {
    for second in ["A.txt", "./a.txt", "a.txt/"] {
        let root = TestRoot::new();
        let zip = root.0.join("duplicate.zip");
        fs::write(&zip, zip_bytes(&[("a.txt", b"first"), (second, b"second")])).unwrap();
        let output = root.0.join("output");
        let result = extract_zip_file_safe(
            &zip,
            &output,
            &mut ExtractionBudget::default(),
            &AtomicBool::new(false),
        );
        assert!(result.unwrap_err().contains("duplicate"));
        assert!(!output.exists());
    }
}

#[test]
fn nested_actions_archives_share_entry_and_decompressed_byte_budgets() {
    let root = TestRoot::new();
    let inner = zip_bytes(&[("first.bin", &[7; 32]), ("second.bin", &[8; 32])]);
    let outer = zip_bytes(&[("package.zip", &inner)]);
    let outer_path = root.0.join("outer.zip");
    fs::write(&outer_path, outer).unwrap();
    let cancelled = AtomicBool::new(false);
    for (index, mut budget) in [
        ExtractionBudget::with_limits(2, 1_000_000, 1_000_000),
        ExtractionBudget::with_limits(10, inner.len() as u64 + 63, 1_000_000),
    ]
    .into_iter()
    .enumerate()
    {
        let artifact = root.0.join(format!("artifact-{index}"));
        extract_zip_file_safe(&outer_path, &artifact, &mut budget, &cancelled).unwrap();
        let output = root.0.join(format!("extracted-{index}"));
        assert!(
            extract_zip_file_safe(
                &artifact.join("package.zip"),
                &output,
                &mut budget,
                &cancelled
            )
            .is_err()
        );
        assert!(!output.exists());
    }
}

#[test]
fn quota_failure_cleans_only_its_stage_and_retains_install_and_rollback() {
    let root = TestRoot::new();
    let installed = root.0.join("installed.exe");
    fs::write(&installed, b"installed").unwrap();
    let prior = root.0.join("prior-stage");
    fs::create_dir(&prior).unwrap();
    fs::write(prior.join("rollback.bin"), b"rollback").unwrap();
    let stage = root.0.join("current-stage");
    sorotte_client_app::app_boundary::persistence::create_private_directory(&stage).unwrap();
    let mut bytes = zip_bytes(&[("sorotte-gui.exe", b"payload")]);
    let central = bytes
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    bytes[central + 24..central + 28]
        .copy_from_slice(&(update_limits::ARCHIVE_BYTES as u32 + 1).to_le_bytes());
    let digest = lowercase_hex(Sha256::digest(&bytes));
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .into_bytes();
    response.extend(bytes);
    let (url, worker) = serve(response);
    let candidate = UpdateCandidate {
        channel: UpdateChannel::Stable,
        version: "0.2.9".to_owned(),
        git_sha: None,
        created_at_utc: "2026-09-05T00:00:00Z".to_owned(),
        target: SOROTTE_GUI_TARGET.to_owned(),
        package: "sorotte-gui-0.2.9-windows-x86_64.zip".to_owned(),
        sha256: digest,
        download_url: url,
        details_url: None,
        source: UpdateCandidateSource::ReleaseAsset,
    };
    let error = stage_update_payload(&candidate, &stage, &AtomicBool::new(false))
        .map_err(|error| cleanup_failed_stage_dir(&stage, error))
        .unwrap_err();
    assert!(error.contains("byte budget"));
    assert!(!stage.exists());
    assert_eq!(fs::read(installed).unwrap(), b"installed");
    assert_eq!(fs::read(prior.join("rollback.bin")).unwrap(), b"rollback");
    worker.join().unwrap();
}
