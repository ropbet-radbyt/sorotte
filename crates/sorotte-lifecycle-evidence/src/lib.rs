//! Privacy-safe, opt-in lifecycle evidence emitted by packaged Sorotte processes.
//!
//! The release harness enables this recorder with environment variables and gives
//! every process a unique output file. Records deliberately accept only bounded
//! tokens and numeric generation identities: raw media names, URLs, filesystem
//! paths, room names, usernames, and protocol payloads cannot enter the schema.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;
pub const SCHEMA_KIND: &str = "sorotte-playback-lifecycle-evidence";
pub const EVIDENCE_PATH_ENV: &str = "SOROTTE_LIFECYCLE_EVIDENCE_PATH";
pub const RUN_ID_ENV: &str = "SOROTTE_LIFECYCLE_RUN_ID";
pub const EMITTER_ENV: &str = "SOROTTE_LIFECYCLE_EMITTER";

const MAX_TOKEN_LEN: usize = 128;
const MAX_COMPONENT_ROLES: usize = 8;
const MAX_IDENTITIES: usize = 16;
const MAX_PREDECESSORS: usize = 16;

static GLOBAL_RECORDER: OnceLock<Option<LifecycleEvidenceRecorder>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessRole {
    Server,
    Client,
    Gui,
    Player,
    Proxy,
    Harness,
    Oracle,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetKind {
    None,
    ProcessBoundary,
    ProtocolMessage,
    ServerState,
    PlayerCommand,
    PlayerState,
    GuiProjection,
    FaultBoundary,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    Startup,
    Shutdown,
    LocalInput,
    RemoteEvent,
    PlayerEvent,
    Timer,
    Fault,
    Recovery,
    Internal,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    Observed,
    Submitted,
    Accepted,
    Committed,
    Applied,
    Rejected,
    Superseded,
    Failed,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessInventorySpec {
    binary_role: ProcessRole,
    component_roles: BTreeSet<ProcessRole>,
}

impl ProcessInventorySpec {
    pub fn new(
        binary_role: ProcessRole,
        component_roles: impl IntoIterator<Item = ProcessRole>,
    ) -> Result<Self, EvidenceError> {
        let component_roles = component_roles.into_iter().collect::<BTreeSet<_>>();
        if component_roles.is_empty() {
            return Err(EvidenceError::EmptyComponentRoles);
        }
        if component_roles.len() > MAX_COMPONENT_ROLES {
            return Err(EvidenceError::TooManyComponentRoles {
                count: component_roles.len(),
                maximum: MAX_COMPONENT_ROLES,
            });
        }
        if !component_roles.contains(&binary_role) {
            return Err(EvidenceError::BinaryRoleNotDeclared { binary_role });
        }
        Ok(Self {
            binary_role,
            component_roles,
        })
    }

    fn component_roles(&self) -> Vec<ProcessRole> {
        self.component_roles.iter().copied().collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionObservation {
    process_role: ProcessRole,
    subject: String,
    machine: String,
    transition: String,
    causal_predecessors: Vec<String>,
    identities: BTreeMap<String, u64>,
    target_kind: TargetKind,
    trigger: Trigger,
    authority_before: String,
    authority_after: String,
    expected_effect: String,
    observed_effect: String,
    disposition: Disposition,
    deadline_ms: Option<u64>,
    deadline_expired: bool,
}

impl TransitionObservation {
    pub fn new(
        process_role: ProcessRole,
        subject: impl Into<String>,
        machine: impl Into<String>,
        transition: impl Into<String>,
    ) -> Self {
        Self {
            process_role,
            subject: subject.into(),
            machine: machine.into(),
            transition: transition.into(),
            causal_predecessors: Vec::new(),
            identities: BTreeMap::new(),
            target_kind: TargetKind::None,
            trigger: Trigger::Internal,
            authority_before: "unknown".to_owned(),
            authority_after: "unknown".to_owned(),
            expected_effect: "observation-only".to_owned(),
            observed_effect: "observed".to_owned(),
            disposition: Disposition::Observed,
            deadline_ms: None,
            deadline_expired: false,
        }
    }

    pub fn causal_predecessor(mut self, event_id: impl Into<String>) -> Self {
        self.causal_predecessors.push(event_id.into());
        self
    }

    pub fn identity(mut self, name: impl Into<String>, value: u64) -> Self {
        self.identities.insert(name.into(), value);
        self
    }

    pub fn target(mut self, target_kind: TargetKind) -> Self {
        self.target_kind = target_kind;
        self
    }

    pub fn triggered_by(mut self, trigger: Trigger) -> Self {
        self.trigger = trigger;
        self
    }

    pub fn authority(mut self, before: impl Into<String>, after: impl Into<String>) -> Self {
        self.authority_before = before.into();
        self.authority_after = after.into();
        self
    }

    pub fn effect(mut self, expected: impl Into<String>, observed: impl Into<String>) -> Self {
        self.expected_effect = expected.into();
        self.observed_effect = observed.into();
        self
    }

    pub fn disposition(mut self, disposition: Disposition) -> Self {
        self.disposition = disposition;
        self
    }

    pub fn deadline(mut self, deadline_ms: u64, expired: bool) -> Self {
        self.deadline_ms = Some(deadline_ms);
        self.deadline_expired = expired;
        self
    }

    fn validate(&self, declared_roles: &BTreeSet<ProcessRole>) -> Result<(), EvidenceError> {
        if !declared_roles.contains(&self.process_role) {
            return Err(EvidenceError::UndeclaredProcessRole {
                process_role: self.process_role,
            });
        }
        validate_token("subject", &self.subject)?;
        validate_token("machine", &self.machine)?;
        validate_token("transition", &self.transition)?;
        validate_token("authority_before", &self.authority_before)?;
        validate_token("authority_after", &self.authority_after)?;
        validate_token("expected_effect", &self.expected_effect)?;
        validate_token("observed_effect", &self.observed_effect)?;
        if self.causal_predecessors.len() > MAX_PREDECESSORS {
            return Err(EvidenceError::TooManyCausalPredecessors {
                count: self.causal_predecessors.len(),
                maximum: MAX_PREDECESSORS,
            });
        }
        for predecessor in &self.causal_predecessors {
            validate_token("causal_predecessor", predecessor)?;
        }
        if self.identities.len() > MAX_IDENTITIES {
            return Err(EvidenceError::TooManyIdentities {
                count: self.identities.len(),
                maximum: MAX_IDENTITIES,
            });
        }
        for (name, value) in &self.identities {
            validate_token("identity_name", name)?;
            if *value == 0 {
                return Err(EvidenceError::ZeroIdentity { name: name.clone() });
            }
        }
        if self.deadline_expired && self.deadline_ms.is_none() {
            return Err(EvidenceError::ExpiredWithoutDeadline);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("lifecycle evidence component roles cannot be empty")]
    EmptyComponentRoles,
    #[error("lifecycle evidence declares {count} component roles; maximum is {maximum}")]
    TooManyComponentRoles { count: usize, maximum: usize },
    #[error("binary role {binary_role:?} is not present in component roles")]
    BinaryRoleNotDeclared { binary_role: ProcessRole },
    #[error("transition uses undeclared process role {process_role:?}")]
    UndeclaredProcessRole { process_role: ProcessRole },
    #[error("lifecycle evidence field {field} must not be empty")]
    EmptyToken { field: &'static str },
    #[error("lifecycle evidence field {field} exceeds {maximum} bytes")]
    TokenTooLong { field: &'static str, maximum: usize },
    #[error("lifecycle evidence field {field} contains unsafe token value")]
    UnsafeToken { field: &'static str },
    #[error("lifecycle evidence declares {count} identities; maximum is {maximum}")]
    TooManyIdentities { count: usize, maximum: usize },
    #[error("lifecycle evidence identity {name} must be greater than zero")]
    ZeroIdentity { name: String },
    #[error("lifecycle evidence declares {count} causal predecessors; maximum is {maximum}")]
    TooManyCausalPredecessors { count: usize, maximum: usize },
    #[error("deadline_expired cannot be true without deadline_ms")]
    ExpiredWithoutDeadline,
    #[error("{name} must be set when {EVIDENCE_PATH_ENV} is enabled")]
    MissingEnvironment { name: &'static str },
    #[error("incomplete lifecycle evidence environment: {name} is set without {EVIDENCE_PATH_ENV}")]
    OrphanedEnvironment { name: &'static str },
    #[error("lifecycle evidence output already exists: {0}")]
    OutputAlreadyExists(PathBuf),
    #[error("lifecycle evidence recorder has already been initialized with another configuration")]
    AlreadyInitialized,
    #[error("lifecycle evidence recorder lock is poisoned")]
    Poisoned,
    #[error("failed to resolve the current executable: {0}")]
    CurrentExecutable(std::io::Error),
    #[error("failed to read executable for lifecycle evidence digest: {0}")]
    DigestRead(std::io::Error),
    #[error("lifecycle evidence I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("lifecycle evidence serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct LifecycleEvidenceRecorder {
    inner: Arc<Mutex<RecorderState>>,
}

struct RecorderState {
    writer: BufWriter<File>,
    origin: Instant,
    sequence: u64,
    run_id: String,
    emitter: String,
    declared_roles: BTreeSet<ProcessRole>,
    last_event_id: String,
}

#[derive(Serialize)]
struct CommonRecord<'a> {
    schema_version: u32,
    kind: &'static str,
    record_type: &'static str,
    event_id: &'a str,
    run_id: &'a str,
    monotonic_ns: u64,
    emitter: &'a str,
}

#[derive(Serialize)]
struct ProcessInventoryRecord<'a> {
    #[serde(flatten)]
    common: CommonRecord<'a>,
    binary_role: ProcessRole,
    component_roles: Vec<ProcessRole>,
    product_name: &'static str,
    product_version: &'static str,
    product_digest: &'a str,
}

#[derive(Serialize)]
struct TransitionRecord<'a> {
    #[serde(flatten)]
    common: CommonRecord<'a>,
    process_role: ProcessRole,
    subject: &'a str,
    machine: &'a str,
    transition: &'a str,
    causal_predecessors: &'a [String],
    identities: &'a BTreeMap<String, u64>,
    target_kind: TargetKind,
    trigger: Trigger,
    authority_before: &'a str,
    authority_after: &'a str,
    expected_effect: &'a str,
    observed_effect: &'a str,
    disposition: Disposition,
    deadline_ms: Option<u64>,
    deadline_expired: bool,
}

impl LifecycleEvidenceRecorder {
    pub fn create(
        path: impl AsRef<Path>,
        run_id: impl Into<String>,
        emitter: impl Into<String>,
        inventory: ProcessInventorySpec,
        product_digest: impl Into<String>,
    ) -> Result<Self, EvidenceError> {
        let path = path.as_ref();
        let run_id = run_id.into();
        let emitter = emitter.into();
        let product_digest = product_digest.into();
        validate_token("run_id", &run_id)?;
        validate_token("emitter", &emitter)?;
        validate_digest(&product_digest)?;

        let file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(EvidenceError::OutputAlreadyExists(path.to_path_buf()));
            }
            Err(error) => return Err(EvidenceError::Io(error)),
        };
        let mut state = RecorderState {
            writer: BufWriter::new(file),
            origin: Instant::now(),
            sequence: 1,
            run_id,
            emitter,
            declared_roles: inventory.component_roles.clone(),
            last_event_id: String::new(),
        };
        let event_id = state.event_id();
        let record = ProcessInventoryRecord {
            common: CommonRecord {
                schema_version: SCHEMA_VERSION,
                kind: SCHEMA_KIND,
                record_type: "process-inventory",
                event_id: &event_id,
                run_id: &state.run_id,
                monotonic_ns: 0,
                emitter: &state.emitter,
            },
            binary_role: inventory.binary_role,
            component_roles: inventory.component_roles(),
            product_name: "sorotte",
            product_version: env!("CARGO_PKG_VERSION"),
            product_digest: &product_digest,
        };
        write_record(&mut state.writer, &record)?;
        state.last_event_id = event_id;
        state.sequence += 1;
        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
        })
    }

    pub fn emit(&self, mut observation: TransitionObservation) -> Result<String, EvidenceError> {
        let mut state = self.inner.lock().map_err(|_| EvidenceError::Poisoned)?;
        observation.validate(&state.declared_roles)?;
        if !observation
            .causal_predecessors
            .iter()
            .any(|predecessor| predecessor == &state.last_event_id)
        {
            observation
                .causal_predecessors
                .insert(0, state.last_event_id.clone());
        }
        if observation.causal_predecessors.len() > MAX_PREDECESSORS {
            return Err(EvidenceError::TooManyCausalPredecessors {
                count: observation.causal_predecessors.len(),
                maximum: MAX_PREDECESSORS,
            });
        }
        let event_id = state.event_id();
        let monotonic_ns = state.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        let run_id = state.run_id.clone();
        let emitter = state.emitter.clone();
        let record = TransitionRecord {
            common: CommonRecord {
                schema_version: SCHEMA_VERSION,
                kind: SCHEMA_KIND,
                record_type: "transition",
                event_id: &event_id,
                run_id: &run_id,
                monotonic_ns,
                emitter: &emitter,
            },
            process_role: observation.process_role,
            subject: &observation.subject,
            machine: &observation.machine,
            transition: &observation.transition,
            causal_predecessors: &observation.causal_predecessors,
            identities: &observation.identities,
            target_kind: observation.target_kind,
            trigger: observation.trigger,
            authority_before: &observation.authority_before,
            authority_after: &observation.authority_after,
            expected_effect: &observation.expected_effect,
            observed_effect: &observation.observed_effect,
            disposition: observation.disposition,
            deadline_ms: observation.deadline_ms,
            deadline_expired: observation.deadline_expired,
        };
        write_record(&mut state.writer, &record)?;
        state.last_event_id.clone_from(&event_id);
        state.sequence += 1;
        Ok(event_id)
    }

    pub fn flush(&self) -> Result<(), EvidenceError> {
        self.inner
            .lock()
            .map_err(|_| EvidenceError::Poisoned)?
            .writer
            .flush()
            .map_err(EvidenceError::Io)
    }
}

impl RecorderState {
    fn event_id(&self) -> String {
        format!("{}.{:08}", self.emitter, self.sequence)
    }
}

pub fn init_global_from_env(inventory: ProcessInventorySpec) -> Result<bool, EvidenceError> {
    let evidence_path = env::var_os(EVIDENCE_PATH_ENV);
    if evidence_path.is_none() {
        for name in [RUN_ID_ENV, EMITTER_ENV] {
            if env::var_os(name).is_some() {
                return Err(EvidenceError::OrphanedEnvironment { name });
            }
        }
        return install_global(None);
    }
    let run_id = required_environment(RUN_ID_ENV)?;
    let emitter = required_environment(EMITTER_ENV)?;
    let product_digest = digest_current_executable()?;
    let recorder = LifecycleEvidenceRecorder::create(
        PathBuf::from(evidence_path.expect("checked above")),
        run_id,
        emitter,
        inventory,
        product_digest,
    )?;
    install_global(Some(recorder))
}

pub fn emit_global(observation: TransitionObservation) -> Result<Option<String>, EvidenceError> {
    match GLOBAL_RECORDER.get().and_then(Option::as_ref) {
        Some(recorder) => recorder.emit(observation).map(Some),
        None => Ok(None),
    }
}

pub fn global_enabled() -> bool {
    GLOBAL_RECORDER.get().is_some_and(Option::is_some)
}

pub fn flush_global() -> Result<(), EvidenceError> {
    match GLOBAL_RECORDER.get().and_then(Option::as_ref) {
        Some(recorder) => recorder.flush(),
        None => Ok(()),
    }
}

fn install_global(recorder: Option<LifecycleEvidenceRecorder>) -> Result<bool, EvidenceError> {
    let enabled = recorder.is_some();
    GLOBAL_RECORDER
        .set(recorder)
        .map_err(|_| EvidenceError::AlreadyInitialized)?;
    Ok(enabled)
}

fn required_environment(name: &'static str) -> Result<String, EvidenceError> {
    env::var(name).map_err(|_| EvidenceError::MissingEnvironment { name })
}

fn digest_current_executable() -> Result<String, EvidenceError> {
    let path = env::current_exe().map_err(EvidenceError::CurrentExecutable)?;
    let mut file = File::open(path).map_err(EvidenceError::DigestRead)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(EvidenceError::DigestRead)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn validate_digest(value: &str) -> Result<(), EvidenceError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(EvidenceError::UnsafeToken {
            field: "product_digest",
        });
    }
    Ok(())
}

fn validate_token(field: &'static str, value: &str) -> Result<(), EvidenceError> {
    if value.is_empty() {
        return Err(EvidenceError::EmptyToken { field });
    }
    if value.len() > MAX_TOKEN_LEN {
        return Err(EvidenceError::TokenTooLong {
            field,
            maximum: MAX_TOKEN_LEN,
        });
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(EvidenceError::UnsafeToken { field });
    }
    Ok(())
}

fn write_record(
    writer: &mut BufWriter<File>,
    record: &impl Serialize,
) -> Result<(), EvidenceError> {
    serde_json::to_writer(&mut *writer, record)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::Value;

    use super::*;

    fn unique_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should follow Unix epoch")
            .as_nanos();
        env::temp_dir().join(format!(
            "sorotte-lifecycle-evidence-{}-{name}-{nonce}.jsonl",
            std::process::id()
        ))
    }

    fn recorder(path: &Path) -> LifecycleEvidenceRecorder {
        LifecycleEvidenceRecorder::create(
            path,
            "run-001",
            "client-a",
            ProcessInventorySpec::new(
                ProcessRole::Client,
                [ProcessRole::Client, ProcessRole::Player],
            )
            .expect("valid inventory"),
            "a".repeat(64),
        )
        .expect("recorder should be created")
    }

    fn read_records(path: &Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .expect("evidence should be readable")
            .lines()
            .map(|line| serde_json::from_str(line).expect("record should be JSON"))
            .collect()
    }

    #[test]
    fn inventory_and_transition_use_complete_safe_schema() {
        let path = unique_path("schema");
        let recorder = recorder(&path);
        let event_id = recorder
            .emit(
                TransitionObservation::new(
                    ProcessRole::Client,
                    "local-session",
                    "canonical-state-transaction",
                    "TX-COMMIT-001",
                )
                .identity("connection-generation", 2)
                .identity("media-generation", 7)
                .target(TargetKind::ProtocolMessage)
                .triggered_by(Trigger::RemoteEvent)
                .authority("server-pending", "server-committed")
                .effect("canonical-state-committed", "canonical-state-committed")
                .disposition(Disposition::Committed)
                .deadline(1_500, false),
            )
            .expect("transition should be emitted");
        recorder.flush().expect("flush should succeed");

        let records = read_records(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["record_type"], "process-inventory");
        assert_eq!(records[0]["binary_role"], "client");
        assert_eq!(records[0]["product_digest"], "a".repeat(64));
        assert_eq!(records[1]["record_type"], "transition");
        assert_eq!(records[1]["event_id"], event_id);
        assert_eq!(records[1]["transition"], "TX-COMMIT-001");
        assert_eq!(records[1]["identities"]["media-generation"], 7);
        assert_eq!(records[1]["causal_predecessors"][0], records[0]["event_id"]);
        assert!(records[1].get("path").is_none());
        assert!(records[1].get("url").is_none());
        std::fs::remove_file(path).expect("test evidence should be removable");
    }

    #[test]
    fn raw_paths_urls_and_whitespace_are_rejected() {
        let path = unique_path("privacy");
        let recorder = recorder(&path);
        for unsafe_subject in [
            "C:\\Users\\person\\video.mkv",
            "https://example.test/media",
            "private room",
        ] {
            let error = recorder
                .emit(TransitionObservation::new(
                    ProcessRole::Client,
                    unsafe_subject,
                    "session",
                    "SESSION-ACTIVE-001",
                ))
                .expect_err("unsafe subject should fail");
            assert!(matches!(error, EvidenceError::UnsafeToken { .. }));
        }
        std::fs::remove_file(path).expect("test evidence should be removable");
    }

    #[test]
    fn undeclared_roles_and_zero_identities_are_rejected() {
        let path = unique_path("role-identity");
        let recorder = recorder(&path);
        let role_error = recorder
            .emit(TransitionObservation::new(
                ProcessRole::Server,
                "room-state",
                "session",
                "SESSION-ACTIVE-001",
            ))
            .expect_err("undeclared role should fail");
        assert!(matches!(
            role_error,
            EvidenceError::UndeclaredProcessRole { .. }
        ));
        let identity_error = recorder
            .emit(
                TransitionObservation::new(
                    ProcessRole::Client,
                    "room-state",
                    "session",
                    "SESSION-ACTIVE-001",
                )
                .identity("connection-generation", 0),
            )
            .expect_err("zero identity should fail");
        assert!(matches!(identity_error, EvidenceError::ZeroIdentity { .. }));
        std::fs::remove_file(path).expect("test evidence should be removable");
    }

    #[test]
    fn concurrent_emission_is_serialized_and_causally_chained() {
        let path = unique_path("concurrency");
        let recorder = Arc::new(recorder(&path));
        let mut threads = Vec::new();
        for _ in 0..4 {
            let recorder = Arc::clone(&recorder);
            threads.push(thread::spawn(move || {
                for _ in 0..25 {
                    recorder
                        .emit(TransitionObservation::new(
                            ProcessRole::Player,
                            "attached-player",
                            "player-transport",
                            "PLAYER-OBSERVE-001",
                        ))
                        .expect("concurrent emit should succeed");
                }
            }));
        }
        for thread in threads {
            thread.join().expect("emitter thread should finish");
        }
        recorder.flush().expect("flush should succeed");

        let records = read_records(&path);
        assert_eq!(records.len(), 101);
        for pair in records.windows(2) {
            assert_eq!(pair[1]["causal_predecessors"][0], pair[0]["event_id"]);
            assert!(pair[1]["monotonic_ns"].as_u64() >= pair[0]["monotonic_ns"].as_u64());
        }
        std::fs::remove_file(path).expect("test evidence should be removable");
    }

    #[test]
    fn output_is_create_new_to_prevent_cross_run_contamination() {
        let path = unique_path("create-new");
        let _recorder = recorder(&path);
        let result = LifecycleEvidenceRecorder::create(
            &path,
            "run-002",
            "client-b",
            ProcessInventorySpec::new(ProcessRole::Client, [ProcessRole::Client])
                .expect("valid inventory"),
            "b".repeat(64),
        );
        let error = result
            .err()
            .expect("existing evidence must not be appended");
        assert!(matches!(error, EvidenceError::OutputAlreadyExists(_)));
        std::fs::remove_file(path).expect("test evidence should be removable");
    }
}
