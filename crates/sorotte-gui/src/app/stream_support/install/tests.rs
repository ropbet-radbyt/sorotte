use super::*;
use crate::app::helper_tools::tests::tool_fixture;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

fn serve_downloads(bodies: Vec<Vec<u8>>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for body in bodies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "download request never arrived");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            assert!(socket.read(&mut [0; 4096]).unwrap() > 0);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        }
    });
    (url, server)
}

fn runtime_archive(executable: &Path) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("bundle/deno.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&fs::read(executable).unwrap()).unwrap();
    zip.finish().unwrap().into_inner()
}

#[test]
fn downloaded_helper_pair_is_validated_together_and_bad_runtime_preserves_the_install() {
    let root = tempfile::tempdir().unwrap();
    let downloader = tool_fixture(root.path(), "yt-dlp");
    let runtime = tool_fixture(root.path(), "deno");
    let (url, server) = serve_downloads(vec![
        fs::read(&downloader).unwrap(),
        runtime_archive(&runtime),
    ]);
    let mut progress = Vec::new();
    install_managed_stream_helper_from_sources(root.path(), &url, &url, None, |event| {
        progress.push(event)
    })
    .unwrap();
    server.join().unwrap();
    assert!(
        progress
            .iter()
            .any(|event| event.label == "Downloading yt-dlp")
    );
    assert!(
        progress
            .iter()
            .any(|event| event.label == "Downloading Deno")
    );
    assert!(
        progress
            .iter()
            .any(|event| event.label == "Validating stream helper binaries")
    );
    let bin = managed_stream_helper_bin_dir(root.path());
    let before_downloader = fs::read(bin.join(managed_downloader_file_name())).unwrap();
    let before_runtime = fs::read(bin.join(managed_js_runtime_file_name())).unwrap();
    let metadata = load_managed_stream_helper_metadata(root.path()).unwrap();
    assert_eq!(metadata.downloader_version.as_deref(), Some("2026.09.01"));
    assert_eq!(metadata.js_runtime_version.as_deref(), Some("deno 2.5.0"));
    let bad = tool_fixture(root.path(), "wrong");
    let (url, server) = serve_downloads(vec![fs::read(downloader).unwrap(), runtime_archive(&bad)]);
    let error = install_managed_stream_helper_from_sources(root.path(), &url, &url, None, |_| {})
        .unwrap_err();
    server.join().unwrap();
    assert!(error.contains("version banner"));
    assert_eq!(
        fs::read(bin.join(managed_downloader_file_name())).unwrap(),
        before_downloader
    );
    assert_eq!(
        fs::read(bin.join(managed_js_runtime_file_name())).unwrap(),
        before_runtime
    );
    assert_eq!(
        load_managed_stream_helper_metadata(root.path())
            .unwrap()
            .js_runtime_version,
        metadata.js_runtime_version
    );
    assert!(!fs::read_dir(bin).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".install-stage")
    }));
}
