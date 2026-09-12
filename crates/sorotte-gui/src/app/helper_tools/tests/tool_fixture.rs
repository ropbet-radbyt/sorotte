// Compiled by helper tests into an isolated, real executable. Its overlay selects behavior
// so copies continue behaving as the selected tool after a managed-install rename.
use std::{
    io::{Read, Seek, SeekFrom, Write},
    time::Duration,
};

fn main() {
    let mut image = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    image.seek(SeekFrom::End(-128)).unwrap();
    let mut tail = String::new();
    let mut bytes = Vec::new();
    image.read_to_end(&mut bytes).unwrap();
    tail.push_str(&String::from_utf8_lossy(&bytes));
    let mode = tail.rsplit("SOROTTE_FIXTURE:").next().unwrap().trim();
    match mode {
        "probe-full" => println!("\nffprobe version 8.0\nconfiguration details"),
        "probe-unterminated" => print!("ffprobe version 8.0-no-newline"),
        "probe-fail" => { eprintln!("probe failure"); std::process::exit(23); }
        "empty" => {},
        "yt-dlp" => println!("2026.09.01"),
        "deno" => println!("deno 2.5.0\nv8 14.0"),
        "ffmpeg" => println!("ffmpeg version 8.0"),
        "ffprobe" => println!("ffprobe version 8.0"),
        "wrong" => println!("Python 3.14.0"),
        "fail" => {
            eprintln!("fixture failure");
            std::process::exit(17);
        }
        "hang" => std::thread::sleep(Duration::from_secs(120)),
        "flood" => {
            let _ = std::io::stdout().write_all(&vec![b'x'; 2 * 1024 * 1024]);
        }
        _ => std::process::exit(19),
    }
}
