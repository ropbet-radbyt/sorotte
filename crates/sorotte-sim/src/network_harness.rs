use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BurstStall {
    pub after_body_bytes: usize,
    pub duration: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkFaultProfile {
    pub first_byte_delay: Duration,
    pub range_response_delay: Duration,
    pub bytes_per_second: Option<u64>,
    pub burst_stalls: Vec<BurstStall>,
    pub disconnect_after_body_bytes: Option<usize>,
    /// Limits an early-disconnect fault to the first N requests for a path.
    /// Zero means that `disconnect_after_body_bytes` applies to every request.
    pub temporary_disconnect_requests: usize,
}

impl Default for NetworkFaultProfile {
    fn default() -> Self {
        Self {
            first_byte_delay: Duration::ZERO,
            range_response_delay: Duration::ZERO,
            bytes_per_second: None,
            burst_stalls: Vec::new(),
            disconnect_after_body_bytes: None,
            temporary_disconnect_requests: 0,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HttpMediaFixture {
    pub content_type: String,
    pub body: Vec<u8>,
    pub seekable: bool,
    pub fault_profile: NetworkFaultProfile,
}

impl std::fmt::Debug for HttpMediaFixture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HttpMediaFixture")
            .field("content_type", &self.content_type)
            .field("body_bytes", &self.body.len())
            .field("seekable", &self.seekable)
            .field("fault_profile", &self.fault_profile)
            .finish()
    }
}

impl HttpMediaFixture {
    pub fn static_bytes(content_type: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            content_type: content_type.into(),
            body: body.into(),
            seekable: true,
            fault_profile: NetworkFaultProfile::default(),
        }
    }

    pub fn non_seekable(mut self) -> Self {
        self.seekable = false;
        self
    }

    pub fn with_faults(mut self, fault_profile: NetworkFaultProfile) -> Self {
        self.fault_profile = fault_profile;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequestRecord {
    pub path: String,
    pub range_start: Option<usize>,
    pub range_end_inclusive: Option<usize>,
    pub status_code: u16,
    pub advertised_body_bytes: usize,
    pub transmitted_body_bytes: usize,
    pub disconnected_early: bool,
}

#[derive(Debug)]
pub struct FaultInjectingHttpServer {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<HttpRequestRecord>>>,
    accept_thread: Option<JoinHandle<()>>,
}

impl FaultInjectingHttpServer {
    pub fn start(fixtures: BTreeMap<String, HttpMediaFixture>) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let fixtures = Arc::new(fixtures);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_counts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_fixtures = Arc::clone(&fixtures);
        let thread_requests = Arc::clone(&requests);
        let thread_request_counts = Arc::clone(&request_counts);
        let thread_shutdown = Arc::clone(&shutdown);
        let accept_thread = thread::Builder::new()
            .name("sorotte-fault-http".to_owned())
            .spawn(move || {
                while !thread_shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let fixtures = Arc::clone(&thread_fixtures);
                            let requests = Arc::clone(&thread_requests);
                            let request_counts = Arc::clone(&thread_request_counts);
                            thread::spawn(move || {
                                if let Some(record) =
                                    handle_connection(stream, &fixtures, &request_counts)
                                {
                                    requests.lock().expect("request log poisoned").push(record);
                                }
                            });
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            address,
            shutdown,
            requests,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn url(&self, path: &str) -> String {
        let path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        format!("http://{}{path}", self.address)
    }

    pub fn requests(&self) -> Vec<HttpRequestRecord> {
        self.requests.lock().expect("request log poisoned").clone()
    }

    pub fn wait_for_requests(&self, count: usize, timeout: Duration) -> bool {
        let started = std::time::Instant::now();
        while started.elapsed() < timeout {
            if self.requests.lock().expect("request log poisoned").len() >= count {
                return true;
            }
            thread::sleep(Duration::from_millis(2));
        }
        false
    }
}

impl Drop for FaultInjectingHttpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    fixtures: &BTreeMap<String, HttpMediaFixture>,
    request_counts: &Mutex<BTreeMap<String, usize>>,
) -> Option<HttpRequestRecord> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let request = read_request_headers(&mut stream).ok()?;
    let (path, range_start, range_end) = parse_request(&request)?;
    let Some(fixture) = fixtures.get(&path) else {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return Some(HttpRequestRecord {
            path,
            range_start,
            range_end_inclusive: range_end,
            status_code: 404,
            advertised_body_bytes: 0,
            transmitted_body_bytes: 0,
            disconnected_early: false,
        });
    };
    let request_ordinal = {
        let mut counts = request_counts.lock().expect("request counter poisoned");
        let count = counts.entry(path.clone()).or_default();
        *count = count.saturating_add(1);
        *count
    };

    let body_len = fixture.body.len();
    let requested_start = range_start.unwrap_or(0).min(body_len);
    let requested_end = range_end
        .unwrap_or_else(|| body_len.saturating_sub(1))
        .min(body_len.saturating_sub(1));
    let partial = fixture.seekable && range_start.is_some() && requested_start <= requested_end;
    let (status_code, body) = if partial {
        (206, &fixture.body[requested_start..=requested_end])
    } else {
        (200, fixture.body.as_slice())
    };

    if partial && !fixture.fault_profile.range_response_delay.is_zero() {
        thread::sleep(fixture.fault_profile.range_response_delay);
    }
    if !fixture.fault_profile.first_byte_delay.is_zero() {
        thread::sleep(fixture.fault_profile.first_byte_delay);
    }
    let mut headers = format!(
        "HTTP/1.1 {status_code} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        if status_code == 206 {
            "Partial Content"
        } else {
            "OK"
        },
        fixture.content_type,
        body.len(),
    );
    if fixture.seekable {
        headers.push_str("Accept-Ranges: bytes\r\n");
    }
    if partial {
        headers.push_str(&format!(
            "Content-Range: bytes {requested_start}-{requested_end}/{body_len}\r\n"
        ));
    }
    headers.push_str("\r\n");
    if stream.write_all(headers.as_bytes()).is_err() {
        return None;
    }

    let transmitted =
        transmit_faulted_body(&mut stream, body, &fixture.fault_profile, request_ordinal);
    Some(HttpRequestRecord {
        path,
        range_start,
        range_end_inclusive: range_end,
        status_code,
        advertised_body_bytes: body.len(),
        transmitted_body_bytes: transmitted,
        disconnected_early: transmitted < body.len(),
    })
}

fn read_request_headers(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    while request.len() < 64 * 1024 {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    Ok(request)
}

fn parse_request(request: &[u8]) -> Option<(String, Option<usize>, Option<usize>)> {
    let request = String::from_utf8_lossy(request);
    let mut lines = request.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?.split('?').next()?.to_owned();
    let range = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range").then_some(value.trim())
    });
    let (start, end) = range
        .and_then(|range| range.strip_prefix("bytes="))
        .and_then(|range| range.split_once('-'))
        .map(|(start, end)| {
            (
                start.parse::<usize>().ok(),
                (!end.is_empty())
                    .then(|| end.parse::<usize>().ok())
                    .flatten(),
            )
        })
        .unwrap_or((None, None));
    Some((path, start, end))
}

fn transmit_faulted_body(
    stream: &mut TcpStream,
    body: &[u8],
    faults: &NetworkFaultProfile,
    request_ordinal: usize,
) -> usize {
    let disconnect_enabled = faults.temporary_disconnect_requests == 0
        || request_ordinal <= faults.temporary_disconnect_requests;
    let disconnect_at = disconnect_enabled
        .then_some(faults.disconnect_after_body_bytes)
        .flatten()
        .unwrap_or(body.len())
        .min(body.len());
    let chunk_size = faults
        .bytes_per_second
        .map(|rate| (rate / 20).max(1) as usize)
        .unwrap_or(64 * 1024);
    let mut transmitted = 0;
    let mut stalls = faults.burst_stalls.clone();
    stalls.sort_by_key(|stall| stall.after_body_bytes);
    let mut next_stall = 0;
    while transmitted < disconnect_at {
        while stalls
            .get(next_stall)
            .is_some_and(|stall| transmitted >= stall.after_body_bytes)
        {
            thread::sleep(stalls[next_stall].duration);
            next_stall += 1;
        }
        let end = transmitted.saturating_add(chunk_size).min(disconnect_at);
        if stream.write_all(&body[transmitted..end]).is_err() {
            break;
        }
        transmitted = end;
        if faults.bytes_per_second.is_some() && transmitted < disconnect_at {
            thread::sleep(Duration::from_millis(50));
        }
    }
    transmitted
}

pub fn hls_vod_manifest(segment_paths: &[&str], segment_duration_seconds: f64) -> Vec<u8> {
    let mut manifest = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n",
        segment_duration_seconds.ceil().max(1.0) as u64
    );
    for path in segment_paths {
        manifest.push_str(&format!("#EXTINF:{segment_duration_seconds:.3},\n{path}\n"));
    }
    manifest.push_str("#EXT-X-ENDLIST\n");
    manifest.into_bytes()
}

pub fn hls_sliding_window_manifest(
    first_sequence: u64,
    segment_paths: &[&str],
    segment_duration_seconds: f64,
) -> Vec<u8> {
    let mut manifest = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{first_sequence}\n",
        segment_duration_seconds.ceil().max(1.0) as u64
    );
    for path in segment_paths {
        manifest.push_str(&format!("#EXTINF:{segment_duration_seconds:.3},\n{path}\n"));
    }
    manifest.into_bytes()
}

pub fn dash_static_manifest(
    initialization_path: &str,
    media_template: &str,
    duration_seconds: u64,
) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MPD type="static" mediaPresentationDuration="PT{duration_seconds}S" minBufferTime="PT1S" xmlns="urn:mpeg:dash:schema:mpd:2011">
  <Period><AdaptationSet mimeType="video/mp4"><Representation id="v" bandwidth="1000000">
    <SegmentTemplate timescale="1" duration="2" initialization="{initialization_path}" media="{media_template}" startNumber="1" />
  </Representation></AdaptationSet></Period>
</MPD>"#
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_get(address: SocketAddr, path: &str, range: Option<&str>) -> Vec<u8> {
        let mut stream = TcpStream::connect(address).expect("fault server should accept");
        let range = range
            .map(|range| format!("Range: {range}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\n{range}Connection: close\r\n\r\n"
        )
        .expect("request should write");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("response should read");
        response
    }

    #[test]
    fn range_requests_and_nonseekable_fixtures_are_distinct() {
        let fixtures = BTreeMap::from([
            (
                "/seekable.bin".to_owned(),
                HttpMediaFixture::static_bytes("application/octet-stream", b"0123456789".to_vec()),
            ),
            (
                "/live.bin".to_owned(),
                HttpMediaFixture::static_bytes("application/octet-stream", b"abcdefghij".to_vec())
                    .non_seekable(),
            ),
        ]);
        let server = FaultInjectingHttpServer::start(fixtures).unwrap();
        let partial = raw_get(server.address(), "/seekable.bin", Some("bytes=2-5"));
        let live = raw_get(server.address(), "/live.bin", Some("bytes=2-5"));

        assert!(partial.starts_with(b"HTTP/1.1 206"));
        assert!(partial.ends_with(b"2345"));
        assert!(live.starts_with(b"HTTP/1.1 200"));
        assert!(live.ends_with(b"abcdefghij"));
        assert!(server.wait_for_requests(2, Duration::from_secs(1)));
        let requests = server.requests();
        assert_eq!(requests[0].range_start, Some(2));
        assert_eq!(requests[0].status_code, 206);
        assert_eq!(requests[1].status_code, 200);
    }

    #[test]
    fn disconnect_fault_advertises_full_body_but_closes_early() {
        let fixture = HttpMediaFixture::static_bytes("video/mp4", vec![7; 64]).with_faults(
            NetworkFaultProfile {
                disconnect_after_body_bytes: Some(12),
                ..NetworkFaultProfile::default()
            },
        );
        let server =
            FaultInjectingHttpServer::start(BTreeMap::from([("/drop.mp4".to_owned(), fixture)]))
                .unwrap();
        let response = raw_get(server.address(), "/drop.mp4", None);

        assert!(
            response
                .windows(20)
                .any(|window| window == b"Content-Length: 64\r\n")
        );
        assert!(server.wait_for_requests(1, Duration::from_secs(1)));
        let record = &server.requests()[0];
        assert_eq!(record.transmitted_body_bytes, 12);
        assert!(record.disconnected_early);
    }

    #[test]
    fn temporary_connection_loss_recovers_on_a_later_request() {
        let fixture = HttpMediaFixture::static_bytes("video/mp4", vec![3; 32]).with_faults(
            NetworkFaultProfile {
                disconnect_after_body_bytes: Some(5),
                temporary_disconnect_requests: 1,
                ..NetworkFaultProfile::default()
            },
        );
        let server = FaultInjectingHttpServer::start(BTreeMap::from([(
            "/temporary.mp4".to_owned(),
            fixture,
        )]))
        .unwrap();
        let first = raw_get(server.address(), "/temporary.mp4", None);
        let second = raw_get(server.address(), "/temporary.mp4", None);

        assert!(first.len() < second.len());
        assert!(server.wait_for_requests(2, Duration::from_secs(1)));
        let requests = server.requests();
        assert!(requests[0].disconnected_early);
        assert!(!requests[1].disconnected_early);
        assert_eq!(requests[1].transmitted_body_bytes, 32);
    }

    #[test]
    fn hls_dash_and_sliding_window_helpers_emit_deterministic_manifests() {
        let vod = String::from_utf8(hls_vod_manifest(&["a.ts", "b.ts"], 2.0)).unwrap();
        let sliding =
            String::from_utf8(hls_sliding_window_manifest(42, &["42.ts", "43.ts"], 2.0)).unwrap();
        let dash = String::from_utf8(dash_static_manifest("init.mp4", "$Number$.m4s", 10)).unwrap();

        assert!(vod.contains("#EXT-X-PLAYLIST-TYPE:VOD"));
        assert!(vod.contains("#EXT-X-ENDLIST"));
        assert!(sliding.contains("#EXT-X-MEDIA-SEQUENCE:42"));
        assert!(!sliding.contains("#EXT-X-ENDLIST"));
        assert!(dash.contains("media=\"$Number$.m4s\""));
    }
}
