use std::{
    collections::VecDeque,
    io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(any(unix, test))]
use std::io::Read;
#[cfg(unix)]
use std::io::Write;

use serde_json::{Value, json};

use crate::constants::{
    MPV_COMMAND_GET_PROPERTY, MPV_COMMAND_OBSERVE_PROPERTY, MPV_RESPONSE_SUCCESS,
};

pub(crate) trait MpvJsonIpcTransport: Send + Sync {
    fn send_line_until(&mut self, line: &str, deadline: Instant) -> io::Result<()>;
    fn read_line_until(&mut self, line: &mut String, deadline: Instant) -> io::Result<usize>;
}

pub(crate) const MPV_IPC_MAX_LINE_BYTES: usize = 1024 * 1024;
pub(crate) const MPV_IPC_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const MPV_IPC_FINAL_WRITE_TIMEOUT: Duration = Duration::from_millis(100);
const MPV_IPC_COMMAND_QUEUE_CAPACITY: usize = 1;
const MPV_IPC_ACTOR_RESPONSE_GRACE: Duration = Duration::from_millis(100);
static NEXT_MPV_IPC_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, PartialEq, Eq)]
pub enum MpvIpcConnectionEvent {
    Connected { generation: u64 },
    CommandFailed { generation: u64, message: String },
    TimedOut { generation: u64, timeout: Duration },
    Disconnected { generation: u64, reason: String },
}

impl std::fmt::Debug for MpvIpcConnectionEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected { generation } => formatter
                .debug_struct("Connected")
                .field("generation", generation)
                .finish(),
            Self::CommandFailed {
                generation,
                message,
            } => formatter
                .debug_struct("CommandFailed")
                .field("generation", generation)
                .field("message_bytes", &message.len())
                .finish(),
            Self::TimedOut {
                generation,
                timeout,
            } => formatter
                .debug_struct("TimedOut")
                .field("generation", generation)
                .field("timeout", timeout)
                .finish(),
            Self::Disconnected { generation, reason } => formatter
                .debug_struct("Disconnected")
                .field("generation", generation)
                .field("reason_bytes", &reason.len())
                .finish(),
        }
    }
}

pub(crate) struct MpvJsonIpcClient {
    command_tx: mpsc::SyncSender<MpvIpcActorMessage>,
    worker_thread: Option<JoinHandle<()>>,
    command_timeout: Duration,
    pending_nonblocking_command: Option<PendingMpvIpcCommand>,
    pending_nonblocking_completions: VecDeque<SequencedMpvIpcNonblockingCommandCompletion>,
    next_runtime_item_sequence: u64,
    generation: u64,
    healthy: bool,
    pending_events: VecDeque<SequencedMpvIpcEvent>,
    pending_connection_events: VecDeque<MpvIpcConnectionEvent>,
}

impl MpvJsonIpcClient {
    pub(crate) fn new(transport: Box<dyn MpvJsonIpcTransport>) -> Self {
        Self::new_with_command_timeout(transport, MPV_IPC_COMMAND_TIMEOUT)
    }

    pub(crate) fn new_with_command_timeout(
        transport: Box<dyn MpvJsonIpcTransport>,
        command_timeout: Duration,
    ) -> Self {
        let generation = NEXT_MPV_IPC_GENERATION.fetch_add(1, Ordering::Relaxed);
        let (command_tx, command_rx) =
            mpsc::sync_channel::<MpvIpcActorMessage>(MPV_IPC_COMMAND_QUEUE_CAPACITY);
        let worker_thread = std::thread::Builder::new()
            .name(format!("sorotte-mpv-ipc-{generation}"))
            .spawn(move || {
                let mut worker = MpvIpcWorker::new(transport);
                let final_completion_tx = loop {
                    let Ok(message) = command_rx.recv() else {
                        break None;
                    };
                    match message {
                        MpvIpcActorMessage::Command(request) => {
                            let outcome = worker.send_command(
                                request.command,
                                request.deadline,
                                request.timeout,
                            );
                            let connection_is_fatal = outcome
                                .result
                                .as_ref()
                                .is_err_and(MpvIpcCommandFailure::is_connection_fatal);
                            let _ = request.response_tx.send(outcome);
                            if connection_is_fatal {
                                break None;
                            }
                        }
                        MpvIpcActorMessage::FinalCommands {
                            commands,
                            completion_tx,
                        } => {
                            worker.send_final_commands_best_effort(commands);
                            break Some(completion_tx);
                        }
                        MpvIpcActorMessage::Shutdown => break None,
                    }
                };
                drop(worker);
                if let Some(completion_tx) = final_completion_tx {
                    let _ = completion_tx.send(());
                }
            })
            .expect("mpv IPC worker thread should start");
        let mut pending_connection_events = VecDeque::new();
        pending_connection_events.push_back(MpvIpcConnectionEvent::Connected { generation });
        Self {
            command_tx,
            worker_thread: Some(worker_thread),
            command_timeout,
            pending_nonblocking_command: None,
            pending_nonblocking_completions: VecDeque::new(),
            next_runtime_item_sequence: 1,
            generation,
            healthy: true,
            pending_events: VecDeque::new(),
            pending_connection_events,
        }
    }

    pub(crate) fn connect(path: &Path) -> Result<Self, String> {
        let transport = MpvPipeTransport::connect(path)
            .map_err(|err| format!("failed to connect mpv IPC at {}: {err}", path.display()))?;
        Ok(Self::new(Box::new(transport)))
    }

    pub(crate) fn is_healthy(&self) -> bool {
        self.healthy
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn mark_unhealthy_for_test(&mut self, reason: impl Into<String>) {
        self.mark_disconnected(reason.into());
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(all(test, windows))]
    pub(crate) fn connect_with_command_timeout(
        path: &Path,
        command_timeout: Duration,
    ) -> Result<Self, String> {
        let transport = MpvPipeTransport::connect(path)
            .map_err(|err| format!("failed to connect mpv IPC at {}: {err}", path.display()))?;
        Ok(Self::new_with_command_timeout(
            Box::new(transport),
            command_timeout,
        ))
    }

    pub(crate) fn send_command_expect_success(&mut self, command: Value) -> Result<(), String> {
        self.send_command(command).map(|_| ())
    }

    /// Queues final JSON IPC writes and waits for their bounded attempts to finish.
    ///
    /// This is intentionally terminal: the client becomes unhealthy and must not be reused. The
    /// worker does not wait for mpv's responses. Each command receives its own short write
    /// deadline, commands retain their order, and a failed write does not prevent later cleanup
    /// commands from being attempted. The bounded completion wait ensures callers cannot reattach
    /// to an external mpv or exit the process before the cleanup worker has made every attempt.
    pub(crate) fn send_final_commands_best_effort(&mut self, commands: Vec<Value>) {
        self.finish_nonblocking_command();
        if !self.healthy {
            return;
        }
        let command_count = u32::try_from(commands.len()).unwrap_or(u32::MAX);
        let completion_timeout = MPV_IPC_FINAL_WRITE_TIMEOUT
            .saturating_mul(command_count)
            .saturating_add(MPV_IPC_ACTOR_RESPONSE_GRACE);
        let (completion_tx, completion_rx) = mpsc::channel();
        let queued = self
            .command_tx
            .try_send(MpvIpcActorMessage::FinalCommands {
                commands,
                completion_tx,
            })
            .is_ok();
        self.healthy = false;
        if queued && completion_rx.recv_timeout(completion_timeout).is_ok() {
            if let Some(worker_thread) = self.worker_thread.take() {
                let _ = worker_thread.join();
            }
        } else {
            let _ = self.worker_thread.take();
        }
    }

    /// Sends a command whose server-side rejection may indicate an older mpv
    /// command shape. A canonical mpv rejection is returned without emitting a
    /// connection-failure event so the caller can try its compatibility form.
    /// Transport, timeout, protocol, and client-side failures remain recorded.
    pub(crate) fn send_compatibility_probe_expect_success(
        &mut self,
        command: Value,
    ) -> Result<(), MpvIpcCommandFailure> {
        self.send_command_classified(command, true).map(|_| ())
    }

    pub(crate) fn observe_property(
        &mut self,
        observer_id: u64,
        property_name: &str,
    ) -> Result<(), String> {
        self.send_command_expect_success(json!([
            MPV_COMMAND_OBSERVE_PROPERTY,
            observer_id,
            property_name
        ]))
    }

    pub(crate) fn get_property(&mut self, property_name: &str) -> Result<Option<Value>, String> {
        let response = self.send_command(json!([MPV_COMMAND_GET_PROPERTY, property_name]))?;
        Ok(response
            .get("data")
            .cloned()
            .filter(|value| !value.is_null()))
    }

    pub(crate) fn get_property_string(
        &mut self,
        property_name: &str,
    ) -> Result<Option<String>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(Value::as_str)
            .map(ToOwned::to_owned))
    }

    /// Queries a string property without erasing mpv's response classification.
    ///
    /// Callers that need to distinguish a genuine unavailable property from a
    /// transport or protocol failure should use this rather than inspecting a
    /// rendered error message.
    pub(crate) fn get_property_string_classified(
        &mut self,
        property_name: &str,
    ) -> Result<Option<String>, MpvIpcCommandFailure> {
        // The caller receives ordinary server rejections directly and can
        // classify an expected unavailable property. Do not also enqueue a
        // misleading connection-failure event for that handled response.
        // Fatal transport/protocol failures are still recorded regardless of
        // this suppression flag.
        let response =
            self.send_command_classified(json!([MPV_COMMAND_GET_PROPERTY, property_name]), true)?;
        Ok(response
            .get("data")
            .filter(|value| !value.is_null())
            .and_then(Value::as_str)
            .map(ToOwned::to_owned))
    }

    pub(crate) fn get_property_f64(&mut self, property_name: &str) -> Result<Option<f64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value.as_ref().and_then(Value::as_f64))
    }

    pub(crate) fn get_property_u64(&mut self, property_name: &str) -> Result<Option<u64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(|value| value.as_u64().or_else(|| value.as_i64()?.try_into().ok())))
    }

    pub(crate) fn get_property_i64(&mut self, property_name: &str) -> Result<Option<i64>, String> {
        let value = self.get_property(property_name)?;
        Ok(value
            .as_ref()
            .and_then(|value| value.as_i64().or_else(|| value.as_u64()?.try_into().ok())))
    }

    fn send_command(&mut self, command: Value) -> Result<Value, String> {
        self.send_command_classified(command, false)
            .map_err(MpvIpcCommandFailure::into_message)
    }

    fn send_command_classified(
        &mut self,
        command: Value,
        suppress_server_rejection_event: bool,
    ) -> Result<Value, MpvIpcCommandFailure> {
        self.finish_nonblocking_command();
        if !self.healthy {
            return Err(MpvIpcCommandFailure::disconnected(
                "mpv IPC connection is not connected".to_owned(),
            ));
        }

        let (response_tx, response_rx) = mpsc::channel();
        let now = Instant::now();
        let deadline = now.checked_add(self.command_timeout).unwrap_or(now);
        match self
            .command_tx
            .try_send(MpvIpcActorMessage::Command(MpvIpcWorkerRequest {
                command,
                deadline,
                timeout: self.command_timeout,
                response_tx,
            })) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                let failure = MpvIpcCommandFailure::command_failed(
                    "mpv IPC command queue is full".to_owned(),
                );
                self.record_failure(&failure);
                return Err(failure);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                let failure = MpvIpcCommandFailure::disconnected(
                    "mpv IPC command worker disconnected".to_owned(),
                );
                self.record_failure(&failure);
                return Err(failure);
            }
        }

        let actor_response_timeout = self
            .command_timeout
            .checked_add(MPV_IPC_ACTOR_RESPONSE_GRACE)
            .unwrap_or(self.command_timeout);
        let outcome = match response_rx.recv_timeout(actor_response_timeout) {
            Ok(outcome) => outcome,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let failure = MpvIpcCommandFailure::timed_out(format!(
                    "mpv IPC command timed out after {:.1} seconds",
                    self.command_timeout.as_secs_f64()
                ));
                self.record_failure(&failure);
                return Err(failure);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let failure = MpvIpcCommandFailure::disconnected(
                    "mpv IPC command worker disconnected".to_owned(),
                );
                self.record_failure(&failure);
                return Err(failure);
            }
        };
        self.extend_pending_events(outcome.pending_events);
        match outcome.result {
            Ok(response) => Ok(response),
            Err(failure) => {
                if !(suppress_server_rejection_event && failure.is_server_rejection()) {
                    self.record_failure(&failure);
                }
                Err(failure)
            }
        }
    }

    /// Attempts to enqueue a command without waiting for the worker or mpv response.
    ///
    /// `Ok(true)` means the command was queued. `Ok(false)` means another nonblocking command or
    /// actor message is still in flight and the caller should retry on a later maintenance tick.
    /// Completion events and failures are harvested by the client's ordinary event accessors.
    pub(crate) fn try_send_command_expect_success_nonblocking(
        &mut self,
        command: Value,
        token: u64,
    ) -> Result<bool, String> {
        self.poll_nonblocking_command();
        if self.pending_nonblocking_command.is_some() {
            return Ok(false);
        }
        if !self.healthy {
            return Err("mpv IPC connection is not connected".to_owned());
        }

        let (response_tx, response_rx) = mpsc::channel();
        let now = Instant::now();
        let command_deadline = now.checked_add(self.command_timeout).unwrap_or(now);
        let response_timeout = self
            .command_timeout
            .checked_add(MPV_IPC_ACTOR_RESPONSE_GRACE)
            .unwrap_or(self.command_timeout);
        let response_deadline = now
            .checked_add(response_timeout)
            .unwrap_or(command_deadline);
        let request = MpvIpcWorkerRequest {
            command,
            deadline: command_deadline,
            timeout: self.command_timeout,
            response_tx,
        };
        match self
            .command_tx
            .try_send(MpvIpcActorMessage::Command(request))
        {
            Ok(()) => {
                self.pending_nonblocking_command = Some(PendingMpvIpcCommand {
                    token,
                    response_rx: Mutex::new(response_rx),
                    response_deadline,
                });
                Ok(true)
            }
            Err(mpsc::TrySendError::Full(_)) => Ok(false),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                let failure = MpvIpcCommandFailure::disconnected(
                    "mpv IPC command worker disconnected".to_owned(),
                );
                let message = failure.message.clone();
                self.record_failure(&failure);
                Err(message)
            }
        }
    }

    fn poll_nonblocking_command(&mut self) {
        let completion = match self.pending_nonblocking_command.as_mut() {
            None => return,
            Some(pending) => match pending
                .response_rx
                .get_mut()
                .expect("exclusive IPC client access must not poison its response receiver")
                .try_recv()
            {
                Ok(outcome) => Some(Ok(outcome)),
                Err(mpsc::TryRecvError::Empty) if Instant::now() < pending.response_deadline => {
                    None
                }
                Err(mpsc::TryRecvError::Empty) => Some(Err(mpsc::RecvTimeoutError::Timeout)),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(mpsc::RecvTimeoutError::Disconnected))
                }
            },
        };
        let Some(completion) = completion else {
            return;
        };
        let token = self
            .pending_nonblocking_command
            .take()
            .expect("polled nonblocking IPC command should remain present")
            .token;
        self.record_nonblocking_completion(token, completion);
    }

    fn finish_nonblocking_command(&mut self) {
        let Some(pending) = self.pending_nonblocking_command.take() else {
            return;
        };
        let remaining = pending
            .response_deadline
            .saturating_duration_since(Instant::now());
        let completion = pending
            .response_rx
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv_timeout(remaining);
        self.record_nonblocking_completion(pending.token, completion);
    }

    fn record_nonblocking_completion(
        &mut self,
        token: u64,
        completion: Result<MpvIpcCommandOutcome, mpsc::RecvTimeoutError>,
    ) {
        match completion {
            Ok(outcome) => {
                self.extend_pending_events(outcome.pending_events);
                match outcome.result {
                    Ok(_) => self.push_nonblocking_completion(
                        MpvIpcNonblockingCommandCompletion::Succeeded { token },
                    ),
                    Err(failure) => {
                        let message = failure.message.clone();
                        self.record_failure(&failure);
                        self.push_nonblocking_completion(
                            MpvIpcNonblockingCommandCompletion::Failed { token, message },
                        );
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let failure = MpvIpcCommandFailure::timed_out(format!(
                    "mpv IPC command timed out after {:.1} seconds",
                    self.command_timeout.as_secs_f64()
                ));
                let message = failure.message.clone();
                self.record_failure(&failure);
                self.push_nonblocking_completion(MpvIpcNonblockingCommandCompletion::Failed {
                    token,
                    message,
                });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let failure = MpvIpcCommandFailure::disconnected(
                    "mpv IPC command worker disconnected".to_owned(),
                );
                let message = failure.message.clone();
                self.record_failure(&failure);
                self.push_nonblocking_completion(MpvIpcNonblockingCommandCompletion::Failed {
                    token,
                    message,
                });
            }
        }
    }

    fn next_runtime_item_sequence(&mut self) -> u64 {
        let sequence = self.next_runtime_item_sequence;
        self.next_runtime_item_sequence = self.next_runtime_item_sequence.wrapping_add(1).max(1);
        sequence
    }

    fn extend_pending_events(&mut self, events: impl IntoIterator<Item = Value>) {
        for event in events {
            let sequence = self.next_runtime_item_sequence();
            self.pending_events.push_back(SequencedMpvIpcEvent {
                sequence,
                value: event,
            });
        }
    }

    fn push_nonblocking_completion(&mut self, value: MpvIpcNonblockingCommandCompletion) {
        let sequence = self.next_runtime_item_sequence();
        self.pending_nonblocking_completions
            .push_back(SequencedMpvIpcNonblockingCommandCompletion { sequence, value });
    }

    pub(crate) fn take_pending_events(&mut self) -> Vec<Value> {
        self.poll_nonblocking_command();
        let completion_barrier = self
            .pending_nonblocking_completions
            .front()
            .map(|completion| completion.sequence);
        let mut events = Vec::new();
        while self
            .pending_events
            .front()
            .is_some_and(|event| completion_barrier.is_none_or(|barrier| event.sequence < barrier))
        {
            events.push(
                self.pending_events
                    .pop_front()
                    .expect("the pending-event front was present")
                    .value,
            );
        }
        events
    }

    #[cfg(test)]
    pub(crate) fn take_pending_events_matching(
        &mut self,
        mut predicate: impl FnMut(&Value) -> bool,
    ) -> Vec<Value> {
        self.poll_nonblocking_command();
        let mut matching = Vec::new();
        let mut retained = VecDeque::with_capacity(self.pending_events.len());
        while let Some(event) = self.pending_events.pop_front() {
            if predicate(&event.value) {
                matching.push(event.value);
            } else {
                retained.push_back(event);
            }
        }
        self.pending_events = retained;
        matching
    }

    pub(crate) fn take_nonblocking_runtime_items_matching(
        &mut self,
        mut predicate: impl FnMut(&Value) -> bool,
    ) -> Vec<MpvIpcNonblockingRuntimeItem> {
        self.poll_nonblocking_command();
        let barrier = self
            .pending_events
            .iter()
            .find(|event| !predicate(&event.value))
            .map(|event| event.sequence);
        let mut items = Vec::new();
        while self
            .pending_events
            .front()
            .is_some_and(|event| barrier.is_none_or(|barrier| event.sequence < barrier))
        {
            let event = self
                .pending_events
                .pop_front()
                .expect("the pending-event front was present");
            items.push((
                event.sequence,
                MpvIpcNonblockingRuntimeItem::Event(event.value),
            ));
        }
        while self
            .pending_nonblocking_completions
            .front()
            .is_some_and(|completion| barrier.is_none_or(|barrier| completion.sequence < barrier))
        {
            let completion = self
                .pending_nonblocking_completions
                .pop_front()
                .expect("the pending-completion front was present");
            items.push((
                completion.sequence,
                MpvIpcNonblockingRuntimeItem::Completion(completion.value),
            ));
        }
        items.sort_unstable_by_key(|(sequence, _)| *sequence);
        items.into_iter().map(|(_, item)| item).collect()
    }

    pub(crate) fn take_connection_events(&mut self) -> Vec<MpvIpcConnectionEvent> {
        self.poll_nonblocking_command();
        self.pending_connection_events.drain(..).collect()
    }

    fn record_command_failure(&mut self, message: &str) {
        self.pending_connection_events
            .push_back(MpvIpcConnectionEvent::CommandFailed {
                generation: self.generation,
                message: message.to_owned(),
            });
    }

    fn record_failure(&mut self, failure: &MpvIpcCommandFailure) {
        self.record_command_failure(&failure.message);
        if matches!(&failure.kind, MpvIpcCommandFailureKind::TimedOut) {
            self.pending_connection_events
                .push_back(MpvIpcConnectionEvent::TimedOut {
                    generation: self.generation,
                    timeout: self.command_timeout,
                });
        }
        if failure.is_connection_fatal() {
            self.mark_disconnected(failure.message.clone());
        }
    }

    fn mark_disconnected(&mut self, reason: String) {
        if !self.healthy {
            return;
        }
        self.healthy = false;
        self.pending_connection_events
            .push_back(MpvIpcConnectionEvent::Disconnected {
                generation: self.generation,
                reason,
            });
    }
}

impl Drop for MpvJsonIpcClient {
    fn drop(&mut self) {
        let _ = self.command_tx.try_send(MpvIpcActorMessage::Shutdown);
        if let Some(worker_thread) = self.worker_thread.take() {
            let _ = worker_thread.join();
        }
    }
}

enum MpvIpcActorMessage {
    Command(MpvIpcWorkerRequest),
    FinalCommands {
        commands: Vec<Value>,
        completion_tx: mpsc::Sender<()>,
    },
    Shutdown,
}

struct MpvIpcWorkerRequest {
    command: Value,
    deadline: Instant,
    timeout: Duration,
    response_tx: mpsc::Sender<MpvIpcCommandOutcome>,
}

struct MpvIpcCommandOutcome {
    result: Result<Value, MpvIpcCommandFailure>,
    pending_events: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MpvIpcCommandFailureKind {
    CommandFailed,
    ServerRejected { server_error: String },
    TimedOut,
    Disconnected,
    ProtocolCorruption,
}

pub(crate) struct MpvIpcCommandFailure {
    kind: MpvIpcCommandFailureKind,
    message: String,
}

impl MpvIpcCommandFailure {
    fn command_failed(message: String) -> Self {
        Self {
            kind: MpvIpcCommandFailureKind::CommandFailed,
            message,
        }
    }

    fn server_rejected(request_id: u64, server_error: String) -> Self {
        let message = format!("mpv command failed for request_id={request_id}: {server_error}");
        Self {
            kind: MpvIpcCommandFailureKind::ServerRejected { server_error },
            message,
        }
    }

    fn timed_out(message: String) -> Self {
        Self {
            kind: MpvIpcCommandFailureKind::TimedOut,
            message,
        }
    }

    fn disconnected(message: String) -> Self {
        Self {
            kind: MpvIpcCommandFailureKind::Disconnected,
            message,
        }
    }

    fn protocol_corruption(message: String) -> Self {
        Self {
            kind: MpvIpcCommandFailureKind::ProtocolCorruption,
            message,
        }
    }

    pub(crate) fn is_server_rejection(&self) -> bool {
        matches!(&self.kind, MpvIpcCommandFailureKind::ServerRejected { .. })
    }

    pub(crate) fn is_property_unavailable(&self) -> bool {
        matches!(
            &self.kind,
            MpvIpcCommandFailureKind::ServerRejected { server_error }
                if server_error == crate::constants::MPV_RESPONSE_PROPERTY_UNAVAILABLE
        )
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn into_message(self) -> String {
        self.message
    }

    fn is_connection_fatal(&self) -> bool {
        matches!(
            &self.kind,
            MpvIpcCommandFailureKind::TimedOut
                | MpvIpcCommandFailureKind::Disconnected
                | MpvIpcCommandFailureKind::ProtocolCorruption
        )
    }

    fn from_read_error(error: io::Error, timeout: Duration) -> Self {
        if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            return Self::timed_out(format!(
                "mpv IPC command timed out after {:.1} seconds",
                timeout.as_secs_f64()
            ));
        }
        Self::disconnected(format!("failed to read mpv IPC response: {error}"))
    }
}

struct MpvIpcWorker {
    transport: Box<dyn MpvJsonIpcTransport>,
    next_request_id: u64,
}

impl MpvIpcWorker {
    fn new(transport: Box<dyn MpvJsonIpcTransport>) -> Self {
        Self {
            transport,
            next_request_id: 1,
        }
    }

    fn send_command(
        &mut self,
        command: Value,
        deadline: Instant,
        timeout: Duration,
    ) -> MpvIpcCommandOutcome {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let mut pending_events = Vec::new();

        let request = json!({
            "command": command,
            "request_id": request_id,
        });
        let mut line = match serde_json::to_string(&request) {
            Ok(line) => line,
            Err(err) => {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::command_failed(format!(
                        "failed to serialize mpv IPC request: {err}"
                    ))),
                    pending_events,
                };
            }
        };
        line.push('\n');
        if remaining_until(deadline).is_err() {
            return MpvIpcCommandOutcome {
                result: Err(MpvIpcCommandFailure::timed_out(format!(
                    "mpv IPC command timed out after {:.1} seconds",
                    timeout.as_secs_f64()
                ))),
                pending_events,
            };
        }
        if let Err(err) = self.transport.send_line_until(&line, deadline) {
            return MpvIpcCommandOutcome {
                result: Err(
                    if matches!(
                        err.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) {
                        MpvIpcCommandFailure::timed_out(format!(
                            "mpv IPC command timed out after {:.1} seconds",
                            timeout.as_secs_f64()
                        ))
                    } else {
                        MpvIpcCommandFailure::disconnected(format!(
                            "failed to write mpv IPC request: {err}"
                        ))
                    },
                ),
                pending_events,
            };
        }

        let mut response_line = String::new();
        loop {
            let bytes_read = match self.transport.read_line_until(&mut response_line, deadline) {
                Ok(bytes_read) => bytes_read,
                Err(err) => {
                    return MpvIpcCommandOutcome {
                        result: Err(MpvIpcCommandFailure::from_read_error(err, timeout)),
                        pending_events,
                    };
                }
            };
            if bytes_read == 0 {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::disconnected(format!(
                        "unexpected EOF while waiting for mpv IPC response (request_id={request_id})"
                    ))),
                    pending_events,
                };
            }
            if let Err(err) = validate_mpv_ipc_line_len(response_line.as_bytes()) {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::protocol_corruption(err)),
                    pending_events,
                };
            }

            let trimmed = response_line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }

            let parsed: Value = match serde_json::from_str(trimmed) {
                Ok(parsed) => parsed,
                Err(err) => {
                    return MpvIpcCommandOutcome {
                        result: Err(MpvIpcCommandFailure::protocol_corruption(format!(
                            "invalid mpv IPC JSON line ({} bytes): {err}",
                            trimmed.len()
                        ))),
                        pending_events,
                    };
                }
            };
            if parsed.get("event").and_then(Value::as_str).is_some() {
                pending_events.push(parsed);
                continue;
            }
            let Some(parsed_request_id) = parsed.get("request_id").and_then(Value::as_u64) else {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::protocol_corruption(format!(
                        "mpv IPC response omitted request_id while waiting for request_id={request_id}"
                    ))),
                    pending_events,
                };
            };
            if parsed_request_id != request_id {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::protocol_corruption(format!(
                        "mpv IPC response request_id mismatch: expected {request_id}, received {parsed_request_id}"
                    ))),
                    pending_events,
                };
            }

            let Some(error) = parsed.get("error").and_then(Value::as_str) else {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::protocol_corruption(format!(
                        "mpv IPC response omitted error for request_id={request_id}"
                    ))),
                    pending_events,
                };
            };
            if error != MPV_RESPONSE_SUCCESS {
                return MpvIpcCommandOutcome {
                    result: Err(MpvIpcCommandFailure::server_rejected(
                        request_id,
                        error.to_owned(),
                    )),
                    pending_events,
                };
            }

            return MpvIpcCommandOutcome {
                result: Ok(parsed),
                pending_events,
            };
        }
    }

    fn send_final_commands_best_effort(&mut self, commands: Vec<Value>) {
        for command in commands {
            let request_id = self.next_request_id;
            self.next_request_id = self.next_request_id.wrapping_add(1);
            let request = json!({
                "command": command,
                "request_id": request_id,
            });
            let Ok(mut line) = serde_json::to_string(&request) else {
                continue;
            };
            line.push('\n');
            let deadline = Instant::now()
                .checked_add(MPV_IPC_FINAL_WRITE_TIMEOUT)
                .unwrap_or_else(Instant::now);
            let _ = self.transport.send_line_until(&line, deadline);
        }
    }
}

struct PendingMpvIpcCommand {
    token: u64,
    response_rx: Mutex<mpsc::Receiver<MpvIpcCommandOutcome>>,
    response_deadline: Instant,
}

struct SequencedMpvIpcEvent {
    sequence: u64,
    value: Value,
}

struct SequencedMpvIpcNonblockingCommandCompletion {
    sequence: u64,
    value: MpvIpcNonblockingCommandCompletion,
}

pub(crate) enum MpvIpcNonblockingCommandCompletion {
    Succeeded { token: u64 },
    Failed { token: u64, message: String },
}

pub(crate) enum MpvIpcNonblockingRuntimeItem {
    Event(Value),
    Completion(MpvIpcNonblockingCommandCompletion),
}

struct MpvPipeTransport {
    stream: MpvPipeStream,
    read_buffer: Vec<u8>,
}

impl MpvPipeTransport {
    pub(crate) fn connect(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::net::UnixStream;
            let stream = UnixStream::connect(path)?;
            Ok(Self {
                stream: MpvPipeStream::Unix(stream),
                read_buffer: Vec::new(),
            })
        }

        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

            let stream = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(FILE_FLAG_OVERLAPPED)
                .open(path)?;
            Ok(Self {
                stream: MpvPipeStream::Windows(MpvWindowsPipe { stream }),
                read_buffer: Vec::new(),
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "mpv IPC transport not implemented for this platform",
            ))
        }
    }
}

impl MpvJsonIpcTransport for MpvPipeTransport {
    fn send_line_until(&mut self, line: &str, deadline: Instant) -> io::Result<()> {
        match &mut self.stream {
            #[cfg(unix)]
            MpvPipeStream::Unix(stream) => write_line_to_unix_stream(stream, line, deadline),
            #[cfg(windows)]
            MpvPipeStream::Windows(stream) => stream.write_line_until(line, deadline),
        }
    }

    fn read_line_until(&mut self, line: &mut String, deadline: Instant) -> io::Result<usize> {
        match &mut self.stream {
            #[cfg(unix)]
            MpvPipeStream::Unix(stream) => read_line_with(&mut self.read_buffer, line, |chunk| {
                let remaining = remaining_until(deadline)?;
                stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
                stream.read(chunk)
            }),
            #[cfg(windows)]
            MpvPipeStream::Windows(stream) => {
                read_line_with(&mut self.read_buffer, line, |chunk| {
                    stream.read_until(chunk, deadline)
                })
            }
        }
    }
}

enum MpvPipeStream {
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
    #[cfg(windows)]
    Windows(MpvWindowsPipe),
}

#[cfg(unix)]
fn write_line_to_unix_stream(
    stream: &mut std::os::unix::net::UnixStream,
    line: &str,
    deadline: Instant,
) -> io::Result<()> {
    let mut remaining_bytes = line.as_bytes();
    while !remaining_bytes.is_empty() {
        let remaining_time = remaining_until(deadline)?;
        stream.set_write_timeout(Some(remaining_time.max(Duration::from_millis(1))))?;
        let written = stream.write(remaining_bytes)?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write mpv IPC request",
            ));
        }
        remaining_bytes = &remaining_bytes[written..];
    }
    stream.flush()
}

#[cfg(windows)]
struct MpvWindowsPipe {
    stream: std::fs::File,
}

#[cfg(windows)]
impl MpvWindowsPipe {
    fn write_line_until(&mut self, line: &str, deadline: Instant) -> io::Result<()> {
        let mut remaining_bytes = line.as_bytes();
        while !remaining_bytes.is_empty() {
            let written = self.write_until(remaining_bytes, deadline)?;
            if written == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write mpv IPC request",
                ));
            }
            remaining_bytes = &remaining_bytes[written..];
        }
        Ok(())
    }

    fn read_until(&mut self, buffer: &mut [u8], deadline: Instant) -> io::Result<usize> {
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::{
            Foundation::{ERROR_IO_PENDING, HANDLE},
            Storage::FileSystem::ReadFile,
            System::IO::OVERLAPPED,
        };

        let event = MpvWindowsEvent::new()?;
        // SAFETY: zero is the documented initial state for `OVERLAPPED`; the
        // event handle is assigned before the operation starts.
        let mut overlapped = unsafe { std::mem::zeroed::<OVERLAPPED>() };
        overlapped.hEvent = event.handle;
        let handle = self.stream.as_raw_handle() as HANDLE;
        let mut immediately_read = 0_u32;
        // SAFETY: `handle` is an overlapped pipe handle, `buffer` remains live
        // until completion, and `overlapped` remains pinned on this stack.
        let started = unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr(),
                buffer.len().min(u32::MAX as usize) as u32,
                &mut immediately_read,
                &mut overlapped,
            )
        };
        if started != 0 {
            return Ok(immediately_read as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(error);
        }
        wait_for_windows_overlapped(handle, &mut overlapped, deadline).map(|bytes| bytes as usize)
    }

    fn write_until(&mut self, buffer: &[u8], deadline: Instant) -> io::Result<usize> {
        use std::os::windows::io::AsRawHandle;

        use windows_sys::Win32::{
            Foundation::{ERROR_IO_PENDING, HANDLE},
            Storage::FileSystem::WriteFile,
            System::IO::OVERLAPPED,
        };

        let event = MpvWindowsEvent::new()?;
        // SAFETY: zero is the documented initial state for `OVERLAPPED`; the
        // event handle is assigned before the operation starts.
        let mut overlapped = unsafe { std::mem::zeroed::<OVERLAPPED>() };
        overlapped.hEvent = event.handle;
        let handle = self.stream.as_raw_handle() as HANDLE;
        let mut immediately_written = 0_u32;
        // SAFETY: `handle` is an overlapped pipe handle, `buffer` remains live
        // until completion, and `overlapped` remains pinned on this stack.
        let started = unsafe {
            WriteFile(
                handle,
                buffer.as_ptr(),
                buffer.len().min(u32::MAX as usize) as u32,
                &mut immediately_written,
                &mut overlapped,
            )
        };
        if started != 0 {
            return Ok(immediately_written as usize);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(error);
        }
        wait_for_windows_overlapped(handle, &mut overlapped, deadline).map(|bytes| bytes as usize)
    }
}

#[cfg(windows)]
struct MpvWindowsEvent {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl MpvWindowsEvent {
    fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::Threading::CreateEventW;

        // SAFETY: null security attributes and name request a private event.
        let handle = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }
}

#[cfg(windows)]
impl Drop for MpvWindowsEvent {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // SAFETY: `handle` was returned by `CreateEventW` and is closed once.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
fn wait_for_windows_overlapped(
    file_handle: windows_sys::Win32::Foundation::HANDLE,
    overlapped: &mut windows_sys::Win32::System::IO::OVERLAPPED,
    deadline: Instant,
) -> io::Result<u32> {
    use windows_sys::Win32::{
        Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{IO::GetOverlappedResult, Threading::WaitForSingleObject},
    };

    let wait_millis = match remaining_until(deadline) {
        Ok(remaining) => remaining
            .as_millis()
            .saturating_add(1)
            .min((u32::MAX - 1) as u128) as u32,
        Err(error) => {
            cancel_and_wait_for_windows_overlapped(file_handle, overlapped);
            return Err(error);
        }
    };
    // SAFETY: the event belongs to the in-flight `OVERLAPPED` operation.
    let wait_result = unsafe { WaitForSingleObject(overlapped.hEvent, wait_millis) };
    if wait_result == WAIT_OBJECT_0 {
        let mut transferred = 0_u32;
        // SAFETY: the event is signaled, so the operation has completed and
        // the `OVERLAPPED` structure is still valid.
        let succeeded =
            unsafe { GetOverlappedResult(file_handle, overlapped, &mut transferred, 0) };
        return if succeeded != 0 {
            Ok(transferred)
        } else {
            Err(io::Error::last_os_error())
        };
    }

    let wait_error = if wait_result == WAIT_TIMEOUT {
        mpv_ipc_timeout_io_error()
    } else if wait_result == WAIT_FAILED {
        io::Error::last_os_error()
    } else {
        io::Error::other(format!(
            "unexpected wait result for mpv IPC operation: {wait_result}"
        ))
    };

    cancel_and_wait_for_windows_overlapped(file_handle, overlapped);
    Err(wait_error)
}

#[cfg(windows)]
fn cancel_and_wait_for_windows_overlapped(
    file_handle: windows_sys::Win32::Foundation::HANDLE,
    overlapped: &mut windows_sys::Win32::System::IO::OVERLAPPED,
) {
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult};

    // SAFETY: this cancels only the operation identified by `overlapped`.
    let _ = unsafe { CancelIoEx(file_handle, overlapped) };
    let mut ignored = 0_u32;
    // SAFETY: waiting for completion ensures the kernel no longer references
    // the stack `OVERLAPPED` or caller buffer before this function returns.
    let _ = unsafe { GetOverlappedResult(file_handle, overlapped, &mut ignored, 1) };
}

fn decode_line_bytes(bytes: Vec<u8>, line: &mut String) -> io::Result<usize> {
    let decoded = String::from_utf8(bytes).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("mpv IPC response was not valid UTF-8: {err}"),
        )
    })?;
    line.clear();
    line.push_str(&decoded);
    Ok(line.len())
}

fn mpv_ipc_line_len(bytes: &[u8]) -> usize {
    let without_lf = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf).len()
}

fn mpv_ipc_line_too_long_error() -> String {
    format!("mpv IPC line too long: exceeded {MPV_IPC_MAX_LINE_BYTES} bytes")
}

fn validate_mpv_ipc_line_len(bytes: &[u8]) -> Result<(), String> {
    if mpv_ipc_line_len(bytes) > MPV_IPC_MAX_LINE_BYTES {
        Err(mpv_ipc_line_too_long_error())
    } else {
        Ok(())
    }
}

fn mpv_ipc_line_too_long_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, mpv_ipc_line_too_long_error())
}

fn mpv_ipc_timeout_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "mpv IPC read deadline elapsed")
}

fn remaining_until(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(mpv_ipc_timeout_io_error)
}

#[cfg(test)]
pub(crate) fn read_line_from_stream(
    stream: &mut impl Read,
    read_buffer: &mut Vec<u8>,
    line: &mut String,
) -> io::Result<usize> {
    read_line_with(read_buffer, line, |chunk| stream.read(chunk))
}

fn read_line_with<F>(read_buffer: &mut Vec<u8>, line: &mut String, mut read: F) -> io::Result<usize>
where
    F: FnMut(&mut [u8]) -> io::Result<usize>,
{
    loop {
        if let Some(newline_index) = read_buffer.iter().position(|byte| *byte == b'\n') {
            if mpv_ipc_line_len(&read_buffer[..=newline_index]) > MPV_IPC_MAX_LINE_BYTES {
                return Err(mpv_ipc_line_too_long_io_error());
            }
            let remainder = read_buffer.split_off(newline_index + 1);
            let bytes = std::mem::replace(read_buffer, remainder);
            return decode_line_bytes(bytes, line);
        }

        let mut chunk = [0_u8; 8 * 1024];
        match read(&mut chunk) {
            Ok(0) => {
                if read_buffer.is_empty() {
                    line.clear();
                    return Ok(0);
                }
                let bytes = std::mem::take(read_buffer);
                return decode_line_bytes(bytes, line);
            }
            Ok(bytes_read) => {
                read_buffer.extend_from_slice(&chunk[..bytes_read]);
                if !read_buffer.contains(&b'\n') && read_buffer.len() > MPV_IPC_MAX_LINE_BYTES {
                    return Err(mpv_ipc_line_too_long_io_error());
                }
            }
            Err(err) => return Err(err),
        }
    }
}
