//! Test-only bounded timing recorder for the opt-in lifecycle experiment.
use std::cell::RefCell;
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static EVENTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
thread_local! { static CLIENT: RefCell<String> = const { RefCell::new(String::new()) }; }

#[cfg(windows)]
pub(crate) fn start() {
    ORIGIN.get_or_init(Instant::now);
    EVENTS.lock().unwrap().clear();
    ACTIVE.store(true, Ordering::Relaxed);
}
pub(crate) fn client(name: &str) {
    CLIENT.with(|value| *value.borrow_mut() = name.to_owned());
}
pub(crate) fn current_client() -> String {
    CLIENT.with(|value| value.borrow().clone())
}
#[cfg(windows)]
pub(crate) fn mark(label: &str) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let row = serde_json::json!({"stage":label, "at_ms":ORIGIN.get().unwrap().elapsed().as_secs_f64()*1000.0,
        "client":CLIENT.with(|value|value.borrow().clone())});
    EVENTS.lock().unwrap().push(row);
}
pub(crate) struct Span {
    label: &'static str,
    start: Instant,
    at_ms: f64,
    client: String,
    thread: String,
}
pub(crate) fn protocol(direction: &str, line: &str) {
    if !ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return;
    };
    if value.get("State").is_none() && value.get("Set").is_none() {
        return;
    }
    let state = &value["State"];
    let playstate = &state["playstate"];
    let mut packet = value.clone();
    // Test media only; reduce bulky fingerprints to presence for this trace.
    if let Some(file) = packet
        .pointer_mut("/Set/file")
        .and_then(|file| file.as_object_mut())
    {
        let shared = file.contains_key(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY);
        file.remove(sorotte_media_match::MEDIA_MATCH_FILE_PAYLOAD_KEY);
        file.insert("review_has_signature".to_owned(), shared.into());
    }
    EVENTS
        .lock()
        .unwrap()
        .push(serde_json::json!({"stage":format!("protocol:{direction}"),
        "at_ms":ORIGIN.get().unwrap().elapsed().as_secs_f64()*1000.0,
        "client":CLIENT.with(|v|v.borrow().clone()),"playstate":playstate,
        "ignoringOnTheFly":state.get("ignoringOnTheFly"),"packet":packet}));
}
pub(crate) fn span(label: &'static str) -> Option<Span> {
    ACTIVE.load(Ordering::Relaxed).then(|| Span {
        label,
        start: Instant::now(),
        at_ms: ORIGIN.get().unwrap().elapsed().as_secs_f64() * 1000.0,
        client: CLIENT.with(|value| value.borrow().clone()),
        thread: std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned(),
    })
}
impl Drop for Span {
    fn drop(&mut self) {
        let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        // Retain the brief spans as well: they distinguish ordinary work from stalls.
        EVENTS
            .lock()
            .unwrap()
            .push(serde_json::json!({"stage":self.label,
            "at_ms":self.at_ms,"duration_ms":elapsed_ms,"client":self.client,"thread":self.thread}));
    }
}
#[cfg(windows)]
pub(crate) fn save(path: &std::path::Path) {
    let mut events = EVENTS.lock().unwrap().clone();
    events.sort_by(|a, b| {
        a["at_ms"]
            .as_f64()
            .unwrap()
            .total_cmp(&b["at_ms"].as_f64().unwrap())
    });
    std::fs::write(path, serde_json::to_vec_pretty(&events).unwrap()).unwrap();
}
