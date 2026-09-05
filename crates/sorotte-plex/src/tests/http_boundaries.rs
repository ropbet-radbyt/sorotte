use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
};

const TOKEN: &str = "plex-redirect-credential-canary";
const IDENTITY: &str = r#"{"MediaContainer":{"machineIdentifier":"fixture"}}"#;

struct Server {
    url: String,
    received: Arc<Mutex<Vec<bool>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(reply: impl Fn(&str) -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(Mutex::new(Vec::new()));
        let worker_stop = stop.clone();
        let worker_received = received.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Windows accepted sockets inherit the listener's nonblocking mode.
                        // Each fixture connection intentionally uses blocking bounded reads.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut byte = [0];
                        while !request.ends_with(b"\r\n\r\n") && request.len() < 16_384 {
                            if stream.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            request.push(byte[0]);
                        }
                        let request = String::from_utf8_lossy(&request);
                        // Retain only the presence bit so assertion output cannot contain credentials.
                        worker_received
                            .lock()
                            .unwrap()
                            .push(request.contains(TOKEN));
                        let path = request.split_whitespace().nth(1).unwrap_or("/");
                        let response = reply(path);
                        let _ = stream.write_all(response.as_bytes());
                        if response.ends_with("\r\n\r\n")
                            && !response.contains("Content-Length: 0\r\n")
                        {
                            // Header-only quota/status fixtures leave the advertised body
                            // pending until the client rejects it, rather than racing an
                            // invalid premature EOF against delivery of the response head.
                            let _ = stream.read(&mut byte);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            received,
            stop,
            worker: Some(worker),
        }
    }

    fn requests(&self) -> Vec<bool> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn json_response(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn redirect(status: u16, target: &str) -> String {
    format!(
        "HTTP/1.1 {status} Redirect\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

#[test]
fn credentialed_redirect_never_contacts_a_different_port() {
    let target = Server::new(|_| json_response(IDENTITY));
    let target_url = target.url.clone();
    let source = Server::new(move |_| redirect(302, &target_url));
    let client = PlexHttpClient::new("redirect-boundary").unwrap();
    let result = client.server_machine_identifier(&source.url, TOKEN);
    let contacted_other_origin = !target.requests().is_empty();
    assert!(
        !contacted_other_origin,
        "redirect must not contact another origin"
    );
    assert!(
        result.is_err(),
        "cross-origin redirect must return an error"
    );
    let error = result.unwrap_err();
    assert!(!format!("{error} {error:?}").contains(TOKEN));
}

#[test]
fn same_origin_relative_redirects_preserve_credentials_for_supported_statuses() {
    for status in [301, 302, 303, 307, 308] {
        let server = Server::new(move |path| {
            if path == "/identity" {
                json_response(IDENTITY)
            } else {
                redirect(status, "/identity")
            }
        });
        let client = PlexHttpClient::new("relative-redirect").unwrap();
        assert_eq!(
            client
                .server_machine_identifier(&server.url, TOKEN)
                .unwrap(),
            "fixture"
        );
        assert_eq!(server.requests(), [true, true]);
    }
}

#[test]
fn every_credentialed_operation_rejects_cross_origin_redirects() {
    let target = Server::new(|_| json_response(IDENTITY));
    let target_url = target.url.clone();
    let source = Server::new(move |_| redirect(307, &target_url));
    let client = PlexHttpClient::with_base_urls(
        &source.url,
        "https://app.plex.tv/auth",
        "all-paths",
        "Sorotte",
    )
    .unwrap();
    let server = PlexServerConnection {
        name: "fixture".to_owned(),
        machine_identifier: "fixture".to_owned(),
        uri: source.url.clone(),
        access_token: TOKEN.into(),
        owned: true,
        has_local_connection: true,
        connection_kind: PlexServerConnectionKind::Local,
    };
    let config = PlexClientConfig {
        selected_server_url: Some(source.url.clone()),
        selected_server_token: Some(TOKEN.into()),
        ..Default::default()
    };
    let report = PlexTimelineReport {
        rating_key: "1".to_owned(),
        state: PlexTimelineState::Playing,
        time_millis: 0,
        duration_millis: Some(1_000),
    };
    let results = [
        client.discover_servers(&TOKEN.into()).map(|_| ()),
        client.verify_server_connection(&server),
        client
            .metadata_by_rating_key(&source.url, TOKEN, "1")
            .map(|_| ()),
        client
            .search_selected_server_media(&config, "movie", 10)
            .map(|_| ()),
        client
            .search_media_by_file_name(&source.url, TOKEN, "movie.mkv")
            .map(|_| ()),
        client.search_media(&source.url, TOKEN, "movie").map(|_| ()),
        client.report_timeline(&source.url, TOKEN, &report),
    ];
    assert!(results.iter().all(Result::is_err));
    assert!(target.requests().is_empty());
    assert!(source.requests().iter().all(|present| *present));
}

#[test]
fn later_cross_host_redirect_and_redirect_loops_are_bounded() {
    let target = Server::new(|_| json_response(IDENTITY));
    let target_url = target.url.replace("127.0.0.1", "localhost");
    let source = Server::new(move |path| {
        if path == "/second" {
            redirect(308, &target_url)
        } else {
            redirect(301, "/second")
        }
    });
    let client = PlexHttpClient::new("redirect-chain").unwrap();
    assert!(
        client
            .server_machine_identifier(&source.url, TOKEN)
            .is_err()
    );
    assert_eq!(source.requests(), [true, true]);
    assert!(target.requests().is_empty());
    let looping = Server::new(|_| redirect(302, "/loop"));
    let error = client
        .server_machine_identifier(&looping.url, TOKEN)
        .unwrap_err();
    assert_eq!(
        looping.requests().len(),
        10,
        "unexpected redacted loop error: {error}"
    );
}

#[test]
fn canonical_origin_compares_scheme_host_and_effective_port() {
    let parse = |url| reqwest::Url::parse(url).unwrap();
    let origin = parse("https://PLEX.example:443/root");
    assert!(http::same_origin(
        &origin,
        &parse("https://plex.example/relative")
    ));
    for url in [
        "http://plex.example/",
        "https://plex.example:444/",
        "https://other.example/",
        "https://user:password@plex.example/",
    ] {
        assert!(!http::same_origin(&origin, &parse(url)));
    }
    assert!(http::same_origin(
        &parse("http://localhost:80/"),
        &parse("http://localhost/")
    ));
}

#[test]
fn https_redirect_cannot_downgrade_a_credentialed_request() {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
    ensure_rustls_crypto_provider();
    let cert_pem = include_bytes!("../../../../fixtures/tls/test_cert.pem");
    let certificate = CertificateDer::from_pem_slice(cert_pem).unwrap();
    let key =
        PrivateKeyDer::from_pem_slice(include_bytes!("../../../../fixtures/tls/test_privkey.pem"))
            .unwrap();
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let target = Server::new(|_| json_response(IDENTITY));
    let target_url = target.url.clone();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let connection = rustls::ServerConnection::new(Arc::new(server_config)).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, stream);
        let mut buffer = [0; 8192];
        let count = stream.read(&mut buffer).unwrap();
        let credential_received = String::from_utf8_lossy(&buffer[..count]).contains(TOKEN);
        stream
            .write_all(redirect(302, &target_url).as_bytes())
            .unwrap();
        credential_received
    });
    let mut client = PlexHttpClient::new("https-downgrade").unwrap();
    client.client = Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(cert_pem).unwrap())
        .redirect(http::redirect_policy())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let result =
        client.server_machine_identifier(&format!("https://localhost:{}", address.port()), TOKEN);
    assert!(
        server.join().unwrap(),
        "TLS fixture must receive the initial credential"
    );
    assert!(result.is_err());
    assert!(target.requests().is_empty());
}

#[test]
fn metadata_ignores_error_bodies_and_bounds_declared_and_chunked_success_bodies() {
    let client = PlexHttpClient::new("metadata-budget").unwrap();
    let error = Server::new(|_| {
        "HTTP/1.1 503 Unavailable\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n"
            .to_owned()
    });
    let result = client
        .server_machine_identifier(&error.url, TOKEN)
        .unwrap_err();
    assert!(
        result.to_string().contains("HTTP 503"),
        "unexpected redacted error: {result}"
    );
    let declared = Server::new(|_| {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            http::METADATA_LIMIT + 1
        )
    });
    assert!(
        client
            .server_machine_identifier(&declared.url, TOKEN)
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
    let chunked = Server::new(|_| {
        let size = http::METADATA_LIMIT + 1;
        format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{size:X}\r\n{}\r\n0\r\n\r\n",
            " ".repeat(size)
        )
    });
    assert!(
        client
            .server_machine_identifier(&chunked.url, TOKEN)
            .unwrap_err()
            .to_string()
            .contains("byte budget")
    );
}

#[test]
fn a_large_section_inventory_cannot_trigger_unbounded_search_requests() {
    let sections = (0..100)
        .map(|index| serde_json::json!({"key": index.to_string(), "type": "movie"}))
        .collect::<Vec<_>>();
    let body = serde_json::json!({"MediaContainer": {"Directory": sections}}).to_string();
    let server = Server::new(move |path| {
        if path == "/library/sections" {
            json_response(&body)
        } else {
            json_response(r#"{"MediaContainer":{"Metadata":[]}}"#)
        }
    });
    let client = PlexHttpClient::new("search-budget").unwrap();
    let result = client.search_media_by_file_name(&server.url, TOKEN, "missing.mkv");
    assert!(result.unwrap_err().to_string().contains("search budget"));
    assert_eq!(server.requests().len(), 64);
}

#[test]
fn search_body_budget_is_shared_across_individually_valid_responses() {
    let sections = (0..10)
        .map(|index| serde_json::json!({"key": index.to_string(), "type": "movie"}))
        .collect::<Vec<_>>();
    let inventory = serde_json::json!({"MediaContainer": {"Directory": sections}}).to_string();
    let large_empty_response = serde_json::json!({
        "MediaContainer": {"Metadata": []}, "padding": "x".repeat(7 * 1024 * 1024)
    })
    .to_string();
    let server = Server::new(move |path| {
        if path == "/library/sections" {
            json_response(&inventory)
        } else {
            json_response(&large_empty_response)
        }
    });
    let client = PlexHttpClient::new("aggregate-search-budget").unwrap();
    let error = client
        .search_media_by_file_name(&server.url, TOKEN, "missing.mkv")
        .unwrap_err();
    assert!(error.to_string().contains("byte budget"));
    assert_eq!(server.requests().len(), 6);
}

#[test]
fn pin_auth_redirect_policy_is_explicit_and_sends_no_token_header() {
    let server = Server::new(|path| {
        if path == "/pin" {
            json_response(r#"{"id":42,"code":"ABCD","authToken":"fixture-token"}"#)
        } else {
            redirect(303, "/pin")
        }
    });
    let client =
        PlexHttpClient::with_base_urls(&server.url, "https://app.plex.tv/auth", "pins", "Sorotte")
            .unwrap();
    assert_eq!(client.start_auth().unwrap().pin_id, 42);
    assert!(client.poll_auth(42).unwrap().auth_token.is_some());
    assert_eq!(server.requests(), [false, false, false, false]);
    let target_url = server.url.clone();
    let redirecting = Server::new(move |_| redirect(307, &target_url));
    let client = PlexHttpClient::with_base_urls(
        &redirecting.url,
        "https://app.plex.tv/auth",
        "pins",
        "Sorotte",
    )
    .unwrap();
    assert!(client.start_auth().is_err());
    assert!(client.poll_auth(42).is_err());
    assert_eq!(server.requests().len(), 4);
}
