use super::{MAX_DOWNLOAD_BYTES, check_cancelled};
use std::{fs, io::Write, path::Path, sync::atomic::AtomicBool, time::Duration};

/// Stream to a private stage. Cancellation also interrupts a request with no incoming bytes.
pub(in crate::app) fn download_to_path(
    url: &str,
    target: &Path,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
    progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    download_to_path_with_limit(url, target, timeout, cancel, MAX_DOWNLOAD_BYTES, progress)
}

fn download_to_path_with_limit(
    url: &str,
    target: &Path,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
    limit: u64,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    check_cancelled(cancel)?;
    crate::app::remote_services::ensure_rustls_crypto_provider();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let operation = async {
            let client = reqwest::Client::builder()
                .timeout(timeout)
                .user_agent(concat!("sorotte-gui/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| error.to_string())?;
            let mut response = client
                .get(url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|error| format!("helper download failed: {error}"))?;
            let total = response.content_length();
            if total.is_some_and(|size| size > limit) {
                return Err("Helper download exceeds the byte limit.".to_owned());
            }
            let mut file = fs::File::create(target).map_err(|error| error.to_string())?;
            let mut downloaded = 0_u64;
            let mut next_progress = 0;
            while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
                check_cancelled(cancel)?;
                downloaded = downloaded
                    .checked_add(chunk.len() as u64)
                    .filter(|size| *size <= limit)
                    .ok_or_else(|| "Helper download exceeds the byte limit.".to_owned())?;
                file.write_all(&chunk).map_err(|error| error.to_string())?;
                if downloaded >= next_progress {
                    progress(downloaded, total);
                    next_progress = downloaded.saturating_add(1024 * 1024);
                }
            }
            if total.is_some_and(|size| size != downloaded) {
                return Err("Helper download ended before its declared length.".to_owned());
            }
            file.sync_all().map_err(|error| error.to_string())?;
            progress(downloaded, total);
            Ok(())
        };
        tokio::pin!(operation);
        loop {
            tokio::select! {
                result = &mut operation => return result,
                _ = tokio::time::sleep(Duration::from_millis(25)) => check_cancelled(cancel)?,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Read,
        net::TcpListener,
        sync::{Arc, atomic::Ordering, mpsc},
        thread,
    };

    #[test]
    fn declared_and_chunked_downloads_obey_the_same_byte_limit_and_reject_truncation() {
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n28\r\n0123456789012345678901234567890123456789\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 30\r\nConnection: close\r\n\r\nshort",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0_u8; 4096];
                assert!(socket.read(&mut request).unwrap() > 0);
                let _ = socket.write_all(response.as_bytes());
            });
            let root = tempfile::tempdir().unwrap();
            let target = root.path().join("download");
            assert!(
                download_to_path_with_limit(
                    &url,
                    &target,
                    Duration::from_secs(3),
                    None,
                    32,
                    |_, _| {}
                )
                .is_err()
            );
            assert!(fs::metadata(&target).map_or(true, |metadata| metadata.len() <= 32));
            server.join().unwrap();
        }
    }

    #[test]
    fn cancellation_interrupts_an_idle_http_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            assert!(socket.read(&mut [0_u8; 4096]).unwrap() > 0);
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("download");
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            finished_tx
                .send(download_to_path(
                    &url,
                    &target,
                    Duration::from_secs(60),
                    Some(&worker_cancel),
                    |_, _| {},
                ))
                .unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        cancel.store(true, Ordering::Release);
        let result = finished_rx.recv_timeout(Duration::from_secs(2));
        release_tx.send(()).unwrap();
        server.join().unwrap();
        worker.join().unwrap();
        assert!(result.unwrap().unwrap_err().contains("cancelled"));
    }
}
