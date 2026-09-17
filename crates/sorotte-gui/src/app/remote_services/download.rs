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
    runtime.block_on(download_with_policy_async(
        url,
        destination,
        cancelled,
        policy,
        token,
    ))
}

async fn download_with_policy_async(
    url: &str,
    destination: &Path,
    cancelled: &AtomicBool,
    policy: DownloadPolicy,
    token: Option<&str>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(policy.connect)
        .read_timeout(policy.idle)
        .timeout(policy.overall)
        .user_agent(format!("sorotte-gui/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("failed building update download client: {error}"))?;
    let transfer = async {
        let mut request = client.get(url).header("Accept", "application/octet-stream");
        if let Some(token) = token {
            // Authorization uses reqwest's cross-origin credential stripping. Plex's
            // custom-header policy is deliberately separate from CDN asset redirects.
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(|error| {
            format!("failed requesting update package: {}", error.without_url())
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "failed downloading update package: HTTP {}",
                response.status()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > policy.bytes)
        {
            return Err("Update package exceeded its download byte budget.".to_owned());
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| format!("failed creating partial update package: {error}"))?;
        let mut count = 0u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("failed reading update package: {}", error.without_url()))?
        {
            check_cancelled(cancelled)?;
            count = count
                .checked_add(chunk.len() as u64)
                .filter(|&length| length <= policy.bytes)
                .ok_or_else(|| "Update package exceeded its download byte budget.".to_owned())?;
            output
                .write_all(&chunk)
                .map_err(|error| format!("failed writing partial update package: {error}"))?;
            digest.update(&chunk);
        }
        check_cancelled(cancelled)?;
        output
            .flush()
            .and_then(|()| output.sync_all())
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::support::test_temp_dir;
    use std::{
        net::TcpListener,
        sync::{Arc, atomic::Ordering},
        thread,
        time::Instant,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn with_controlled_time(test: impl std::future::Future<Output = ()>) {
        ensure_rustls_crypto_provider();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                // Paused Tokio time otherwise auto-advances when socket I/O
                // is pending. This yielding watchdog keeps the test runnable
                // so only explicit advances consume the download's timeout
                // budget. Its wall-clock bound also covers accept and headers.
                let watchdog = async {
                    let started = Instant::now();
                    loop {
                        assert!(
                            started.elapsed() < Duration::from_secs(10),
                            "controlled HTTP download fixture exceeded its wall-clock watchdog"
                        );
                        tokio::task::yield_now().await;
                    }
                };
                tokio::select! {
                    () = test => {},
                    () = watchdog => {},
                }
            });
    }

    async fn controlled_transfer(
        path: &Path,
        cancelled: Arc<AtomicBool>,
        policy: DownloadPolicy,
    ) -> (
        tokio::net::TcpStream,
        tokio::task::JoinHandle<Result<String, String>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/package", listener.local_addr().unwrap());
        let destination = path.to_owned();
        let transfer = tokio::spawn(async move {
            download_with_policy_async(&url, &destination, &cancelled, policy, None).await
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
            assert!(request.len() < 8192, "unexpected download request size");
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        wait_for_downloaded_bytes(path, 0, &transfer).await;
        (stream, transfer)
    }

    async fn wait_for_downloaded_bytes(
        path: &Path,
        bytes: u64,
        transfer: &tokio::task::JoinHandle<Result<String, String>>,
    ) {
        while !fs::metadata(path).is_ok_and(|metadata| metadata.len() == bytes) {
            assert!(
                !transfer.is_finished(),
                "download ended before receiving {bytes} bytes"
            );
            tokio::task::yield_now().await;
        }
    }

    async fn finish_transfer(
        transfer: tokio::task::JoinHandle<Result<String, String>>,
    ) -> Result<String, String> {
        while !transfer.is_finished() {
            tokio::task::yield_now().await;
        }
        transfer.await.unwrap()
    }

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

    #[test]
    fn concurrent_downloads_keep_payloads_and_cleanup_separate() {
        let mut downloads = thread::scope(|scope| {
            let workers: Vec<_> = (0..4u8)
                .map(|index| {
                    scope.spawn(move || {
                        let root = test_temp_dir("download");
                        let path = root.path().join("package.tmp");
                        let payload = vec![b'a' + index; 4];
                        let body = payload.clone();
                        let (url, worker) = server(move |mut stream| {
                            stream
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
                                .unwrap();
                            stream.write_all(&body).unwrap();
                        });
                        let result = download_with_policy(
                            &url, &path, &AtomicBool::new(false), test_policy(), None,
                        );
                        worker.join().unwrap();
                        assert_eq!(result.unwrap(), lowercase_hex(Sha256::digest(&payload)));
                        (root, payload)
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        let (finished, _) = downloads.pop().unwrap();
        let finished_path = finished.path().to_path_buf();
        drop(finished);
        assert!(!finished_path.exists());
        for (root, payload) in downloads {
            assert_eq!(fs::read(root.path().join("package.tmp")).unwrap(), payload);
            root.close().unwrap();
        }
    }

    #[test]
    fn steadily_progressing_transfer_can_outlive_its_idle_deadline() {
        assert!(DownloadPolicy::default().overall > Duration::from_secs(10));
        let root = test_temp_dir("download");
        let path = root.path().join("package.tmp");
        with_controlled_time(async {
            let (mut stream, transfer) =
                controlled_transfer(&path, Arc::new(AtomicBool::new(false)), test_policy()).await;
            let start = tokio::time::Instant::now();
            for bytes in 1..=4 {
                tokio::time::advance(Duration::from_millis(120)).await;
                stream.write_all(b"1\r\na\r\n").await.unwrap();
                // Receipt, not the sender's write, establishes progress and
                // permits the next clock advance. OS scheduling cannot turn
                // this progressing transfer into an unintended idle one.
                wait_for_downloaded_bytes(&path, bytes, &transfer).await;
            }
            stream.write_all(b"0\r\n\r\n").await.unwrap();
            let digest = finish_transfer(transfer).await.unwrap();
            assert_eq!(start.elapsed(), Duration::from_millis(480));
            assert!(start.elapsed() > test_policy().idle);
            assert_eq!(digest, lowercase_hex(Sha256::digest(b"aaaa")));
            assert_eq!(fs::read(&path).unwrap(), b"aaaa");
        });
        root.close().unwrap();
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
            let root = test_temp_dir("download");
            let path = root.path().join("package.tmp");
            let error =
                download_with_policy(&url, &path, &AtomicBool::new(false), test_policy(), None)
                    .unwrap_err();
            assert!(error.contains("byte budget"));
            assert!(fs::metadata(&path).map_or(true, |metadata| metadata.len() <= 8));
            worker.join().unwrap();
            root.close().unwrap();
        }
    }

    #[test]
    fn cancellation_interrupts_an_idle_body_without_waiting_for_idle_timeout() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let root = test_temp_dir("download");
        let path = root.path().join("package.tmp");
        let mut policy = test_policy();
        policy.idle = Duration::from_secs(10);
        with_controlled_time(async {
            let (stream, transfer) = controlled_transfer(&path, cancelled.clone(), policy).await;
            let start = tokio::time::Instant::now();
            cancelled.store(true, Ordering::Release);
            tokio::time::advance(Duration::from_millis(51)).await;
            let error = finish_transfer(transfer).await.unwrap_err();
            assert!(error.contains("cancelled"), "{error}");
            assert_eq!(start.elapsed(), Duration::from_millis(51));
            assert!(start.elapsed() < policy.idle.min(policy.overall));
            // Keep the peer open until the cancellation result: EOF cannot
            // accidentally satisfy the failure assertion.
            drop(stream);
        });
        root.close().unwrap();
    }

    #[test]
    fn separate_idle_and_overall_deadlines_stop_stalled_or_endless_transfers() {
        let root = test_temp_dir("download");
        let path = root.path().join("package.tmp");
        with_controlled_time(async {
            let policy = test_policy();
            let (stream, transfer) =
                controlled_transfer(&path, Arc::new(AtomicBool::new(false)), policy).await;
            let start = tokio::time::Instant::now();
            tokio::time::advance(Duration::from_millis(299)).await;
            assert!(
                !transfer.is_finished(),
                "body should remain live before the idle deadline"
            );
            tokio::time::advance(Duration::from_millis(2)).await;
            let error = finish_transfer(transfer).await.unwrap_err();
            assert!(
                error.starts_with("failed reading update package:"),
                "{error}"
            );
            assert_eq!(start.elapsed(), Duration::from_millis(301));
            assert!(start.elapsed() < policy.overall);
            assert_eq!(fs::metadata(&path).unwrap().len(), 0);
            drop(stream);
        });
        root.close().unwrap();

        let root = test_temp_dir("download");
        let path = root.path().join("package.tmp");
        let mut policy = test_policy();
        policy.bytes = 100;
        policy.overall = Duration::from_millis(160);
        with_controlled_time(async {
            let (mut stream, transfer) =
                controlled_transfer(&path, Arc::new(AtomicBool::new(false)), policy).await;
            let start = tokio::time::Instant::now();
            for bytes in 1..=5 {
                tokio::time::advance(Duration::from_millis(30)).await;
                stream.write_all(b"1\r\nx\r\n").await.unwrap();
                wait_for_downloaded_bytes(&path, bytes, &transfer).await;
            }
            assert!(!transfer.is_finished());
            tokio::time::advance(Duration::from_millis(11)).await;
            let error = finish_transfer(transfer).await.unwrap_err();
            assert!(
                error == "Update package exceeded its overall download deadline."
                    || error.starts_with("failed reading update package:"),
                "{error}"
            );
            assert_eq!(start.elapsed(), Duration::from_millis(161));
            assert!(start.elapsed() < policy.idle);
            assert_eq!(fs::metadata(&path).unwrap().len(), 5);
            drop(stream);
        });
        root.close().unwrap();
    }
}
