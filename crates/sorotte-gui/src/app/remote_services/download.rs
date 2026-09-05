use super::*;
use update_limits::ARCHIVE_BYTES;

#[derive(Clone, Copy)]
struct DownloadPolicy {
    connect: Duration,
    idle: Duration,
    overall: Duration,
    bytes: u64,
}

impl Default for DownloadPolicy {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            idle: Duration::from_secs(30),
            overall: Duration::from_secs(30 * 60),
            bytes: ARCHIVE_BYTES,
        }
    }
}

/// Runs only on the existing update worker. No complete package is retained in memory.
pub(super) fn download_package(
    url: &str,
    destination: &Path,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let token = env_trimmed(SOROTTE_GUI_GITHUB_TOKEN_ENV);
    download_with_policy(
        url,
        destination,
        cancelled,
        DownloadPolicy::default(),
        token.as_deref(),
    )
}

fn download_with_policy(
    url: &str,
    destination: &Path,
    cancelled: &AtomicBool,
    policy: DownloadPolicy,
    token: Option<&str>,
) -> Result<String, String> {
    check_cancelled(cancelled)?;
    ensure_rustls_crypto_provider();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed creating update download runtime: {error}"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(policy.connect)
            .read_timeout(policy.idle)
            .timeout(policy.overall)
            .user_agent(format!("sorotte-gui/{}", env!("CARGO_PKG_VERSION")))
            .build().map_err(|error| format!("failed building update download client: {error}"))?;
        let transfer = async {
            let mut request = client.get(url).header("Accept", "application/octet-stream");
            if let Some(token) = token {
                // Authorization uses reqwest's cross-origin credential stripping. Plex's
                // custom-header policy is deliberately separate from CDN asset redirects.
                request = request.bearer_auth(token);
            }
            let mut response = request.send().await
                .map_err(|error| format!("failed requesting update package: {}", error.without_url()))?;
            if !response.status().is_success() {
                return Err(format!("failed downloading update package: HTTP {}", response.status()));
            }
            if response.content_length().is_some_and(|length| length > policy.bytes) {
                return Err("Update package exceeded its download byte budget.".to_owned());
            }
            let mut output = fs::OpenOptions::new().write(true).create_new(true).open(destination)
                .map_err(|error| format!("failed creating partial update package: {error}"))?;
            let mut count = 0u64;
            let mut digest = Sha256::new();
            while let Some(chunk) = response.chunk().await
                .map_err(|error| format!("failed reading update package: {}", error.without_url()))? {
                check_cancelled(cancelled)?;
                count = count.checked_add(chunk.len() as u64)
                    .filter(|&length| length <= policy.bytes)
                    .ok_or_else(|| "Update package exceeded its download byte budget.".to_owned())?;
                output.write_all(&chunk).map_err(|error| format!("failed writing partial update package: {error}"))?;
                digest.update(&chunk);
            }
            check_cancelled(cancelled)?;
            output.flush().and_then(|()| output.sync_all())
                .map_err(|error| format!("failed flushing update package: {error}"))?;
            Ok(lowercase_hex(digest.finalize()))
        };
        tokio::pin!(transfer);
        let deadline = tokio::time::sleep(policy.overall);
        tokio::pin!(deadline);
        let mut cancellation_poll = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                result = &mut transfer => return result,
                _ = &mut deadline => return Err("Update package exceeded its overall download deadline.".to_owned()),
                _ = cancellation_poll.tick() => check_cancelled(cancelled)?,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{Arc, atomic::Ordering},
        thread,
        time::Instant,
    };

    fn server(
        reply: impl FnOnce(std::net::TcpStream) + Send + 'static,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/package", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 8192];
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let _ = stream.read(&mut request);
            reply(stream);
        });
        (url, worker)
    }

    fn test_policy() -> DownloadPolicy {
        DownloadPolicy {
            connect: Duration::from_secs(1),
            idle: Duration::from_millis(300),
            overall: Duration::from_secs(3),
            bytes: 8,
        }
    }

    fn destination() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sorotte-download-{}-{nonce}.tmp",
            std::process::id()
        ))
    }

    #[test]
    fn steadily_progressing_transfer_can_outlive_its_idle_deadline() {
        assert!(DownloadPolicy::default().overall > Duration::from_secs(10));
        let (url, worker) = server(|mut stream| {
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            for _ in 0..4 {
                thread::sleep(Duration::from_millis(120));
                let _ = stream.write_all(b"1\r\na\r\n");
            }
            let _ = stream.write_all(b"0\r\n\r\n");
        });
        let path = destination();
        let start = Instant::now();
        let digest =
            download_with_policy(&url, &path, &AtomicBool::new(false), test_policy(), None)
                .unwrap();
        assert!(start.elapsed() > test_policy().idle);
        assert_eq!(digest, lowercase_hex(Sha256::digest(b"aaaa")));
        assert_eq!(fs::read(&path).unwrap(), b"aaaa");
        fs::remove_file(path).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn oversized_declared_chunked_and_lengthless_packages_never_exceed_disk_budget() {
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n9\r\n123456789\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n123456789",
        ] {
            let (url, worker) = server(move |mut stream| {
                let _ = stream.write_all(response.as_bytes());
            });
            let path = destination();
            let error =
                download_with_policy(&url, &path, &AtomicBool::new(false), test_policy(), None)
                    .unwrap_err();
            assert!(error.contains("byte budget"));
            assert!(fs::metadata(&path).map_or(true, |metadata| metadata.len() <= 8));
            let _ = fs::remove_file(path);
            worker.join().unwrap();
        }
    }

    #[test]
    fn cancellation_interrupts_an_idle_body_without_waiting_for_idle_timeout() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let server_cancelled = cancelled.clone();
        let (url, worker) = server(move |mut stream| {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n");
            thread::sleep(Duration::from_millis(100));
            server_cancelled.store(true, Ordering::Release);
            thread::sleep(Duration::from_millis(300));
        });
        let path = destination();
        let mut policy = test_policy();
        policy.idle = Duration::from_secs(10);
        let start = Instant::now();
        let error = download_with_policy(&url, &path, &cancelled, policy, None).unwrap_err();
        assert!(error.contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
        let _ = fs::remove_file(path);
        worker.join().unwrap();
    }

    #[test]
    fn separate_idle_and_overall_deadlines_stop_stalled_or_endless_transfers() {
        let (url, worker) = server(|mut stream| {
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n");
            thread::sleep(Duration::from_millis(500));
        });
        let path = destination();
        let start = Instant::now();
        assert!(
            download_with_policy(&url, &path, &AtomicBool::new(false), test_policy(), None)
                .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        let _ = fs::remove_file(path);
        worker.join().unwrap();

        let (url, worker) = server(|mut stream| {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            );
            for _ in 0..20 {
                thread::sleep(Duration::from_millis(30));
                if stream.write_all(b"1\r\nx\r\n").is_err() {
                    break;
                }
            }
        });
        let path = destination();
        let mut policy = test_policy();
        policy.bytes = 100;
        policy.overall = Duration::from_millis(160);
        assert!(download_with_policy(&url, &path, &AtomicBool::new(false), policy, None).is_err());
        assert!(fs::metadata(&path).is_ok_and(|metadata| metadata.len() < 20));
        let _ = fs::remove_file(path);
        worker.join().unwrap();
    }
}
