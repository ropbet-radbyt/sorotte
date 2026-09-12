// Windows extraction tests need a real executable, not a shell script. This
// shim delegates to the existing process fixture and stays in its owner's job.
use std::{io::{Read, Seek, SeekFrom}, process::{Command, Stdio}};

fn main() {
    let mut image = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    image.seek(SeekFrom::End(-8)).unwrap();
    let mut size = [0; 8];
    image.read_exact(&mut size).unwrap();
    let size = u64::from_le_bytes(size);
    assert!(size < 64 * 1024);
    image.seek(SeekFrom::End(-(size as i64) - 8)).unwrap();
    let mut config = vec![0; size as usize];
    image.read_exact(&mut config).unwrap();
    let config = String::from_utf8(config).unwrap();
    let values = config.lines().collect::<Vec<_>>();
    assert_eq!(values.len(), 3);
    if values[1] == "arguments" {
        for arg in std::env::args().skip(1) { print!("{arg}\0"); }
        return;
    }
    let status = Command::new(values[0])
        .args(["--exact", "extraction::process::tests::media_tool_process_fixture",
            "--ignored", "--nocapture", "--test-threads=1"])
        .env("SOROTTE_MEDIA_PROCESS_FIXTURE", values[1])
        .env("SOROTTE_MEDIA_PROCESS_MARKER", values[2])
        .stdin(Stdio::null()).status().unwrap();
    std::process::exit(status.code().unwrap_or(82));
}
