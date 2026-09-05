use super::*;
use tokio::sync::oneshot;

const SERVER_COMMAND_QUEUE_CAPACITY: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ServerActorError {
    #[error(transparent)]
    Runtime(#[from] ServerRuntimeError),
    #[error("server actor is unavailable")]
    Unavailable,
    #[error("server actor task failed: {0}")]
    TaskFailed(String),
    #[error("server actor shutdown barrier failed: {barrier}; actor task also failed: {task}")]
    ShutdownFailed { barrier: String, task: String },
}

enum ServerCommand {
    #[cfg(test)]
    PauseForTest {
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    },
    HandleLine {
        client_id: String,
        line: String,
        peer_ip: Option<String>,
        reply: oneshot::Sender<Result<(ServerRuntimeDispatch, bool), ServerRuntimeError>>,
    },
    Disconnect {
        client_id: String,
        reply: oneshot::Sender<Result<Vec<DirectedOutboundLine>, ServerRuntimeError>>,
    },
    CollectDispatch {
        now_seconds: f64,
        reply: oneshot::Sender<Result<ServerRuntimeDispatch, ServerRuntimeError>>,
    },
    TlsServerConfig {
        reply: oneshot::Sender<Option<Arc<ServerConfig>>>,
    },
    Session {
        client_id: String,
        reply: oneshot::Sender<Option<ServerSession>>,
    },
    ServerIgnoringCounter {
        client_id: String,
        reply: oneshot::Sender<u32>,
    },
    TimeNowOverride {
        reply: oneshot::Sender<Option<f64>>,
    },
    FlushPersistence {
        reply: oneshot::Sender<Result<(), ServerRuntimeError>>,
    },
    Shutdown {
        deadline: std::time::Instant,
        reply: oneshot::Sender<Result<(), ServerActorError>>,
    },
}

/// Bounded, cloneable ingress for the single task that owns the server model.
#[derive(Debug, Clone)]
pub struct ServerActorHandle {
    commands: Sender<ServerCommand>,
    persistence_events: broadcast::Sender<ServerPersistenceEvent>,
    persistence_degraded_worker_count: Arc<AtomicUsize>,
    outbound_backpressure_metrics: ServerOutboundBackpressureMetrics,
    pub(crate) network_resources: Arc<resources::NetworkResources>,
    join_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    persistence_controls: Vec<Arc<persistence_actor::WorkerControl>>,
}

impl ServerActorHandle {
    pub fn spawn(mut runtime: ServerRuntime) -> Self {
        let (commands, receiver) = channel(SERVER_COMMAND_QUEUE_CAPACITY);
        let persistence_events = runtime.persistence_events.clone();
        let persistence_degraded_worker_count = runtime.persistence_degraded_worker_count.clone();
        let network_resources = resources::NetworkResources::new(runtime.resource_limits);
        let persistence_controls = runtime
            .room_persistence
            .as_ref()
            .map(|service| service.control())
            .into_iter()
            .chain(
                runtime
                    .stats_persistence
                    .as_ref()
                    .map(|service| service.control()),
            )
            .collect();
        let join_handle = tokio::spawn(async move {
            run_server_actor(&mut runtime, receiver).await;
        });
        Self {
            commands,
            persistence_events,
            persistence_degraded_worker_count,
            outbound_backpressure_metrics: ServerOutboundBackpressureMetrics::default(),
            network_resources,
            join_handle: Arc::new(Mutex::new(Some(join_handle))),
            persistence_controls,
        }
    }

    pub fn subscribe_persistence_events(&self) -> broadcast::Receiver<ServerPersistenceEvent> {
        self.persistence_events.subscribe()
    }

    pub fn persistence_is_degraded(&self) -> bool {
        self.persistence_degraded_worker_count
            .load(Ordering::Acquire)
            > 0
    }

    pub fn outbound_backpressure_snapshot(&self) -> ServerOutboundBackpressureSnapshot {
        self.outbound_backpressure_metrics.snapshot()
    }

    pub fn resource_snapshot(&self) -> ServerResourceSnapshot {
        self.network_resources.snapshot()
    }

    pub(crate) fn outbound_backpressure_metrics(&self) -> ServerOutboundBackpressureMetrics {
        self.outbound_backpressure_metrics.clone()
    }

    pub(crate) async fn handle_line(
        &self,
        client_id: &str,
        line: &str,
        peer_ip: Option<&str>,
    ) -> Result<(ServerRuntimeDispatch, bool), ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::HandleLine {
                client_id: client_id.to_owned(),
                line: line.to_owned(),
                peer_ip: peer_ip.map(str::to_owned),
                reply,
            })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response
            .await
            .map_err(|_| ServerActorError::Unavailable)?
            .map_err(ServerActorError::from)
    }

    pub(crate) async fn disconnect(
        &self,
        client_id: &str,
    ) -> Result<Vec<DirectedOutboundLine>, ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::Disconnect {
                client_id: client_id.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response
            .await
            .map_err(|_| ServerActorError::Unavailable)?
            .map_err(ServerActorError::from)
    }

    pub(crate) async fn collect_dispatch(
        &self,
        now_seconds: f64,
    ) -> Result<ServerRuntimeDispatch, ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::CollectDispatch { now_seconds, reply })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response
            .await
            .map_err(|_| ServerActorError::Unavailable)?
            .map_err(ServerActorError::from)
    }

    pub(crate) async fn tls_server_config(
        &self,
    ) -> Result<Option<Arc<ServerConfig>>, ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::TlsServerConfig { reply })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response.await.map_err(|_| ServerActorError::Unavailable)
    }

    pub async fn session(
        &self,
        client_id: &str,
    ) -> Result<Option<ServerSession>, ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::Session {
                client_id: client_id.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response.await.map_err(|_| ServerActorError::Unavailable)
    }

    /// Returns the current generation counter that a raw protocol peer must
    /// acknowledge before its playback sample can become authoritative.
    /// This is intentionally read-only and is useful to black-box harnesses
    /// that observe the actor through the production network transport.
    pub async fn server_ignoring_counter(&self, client_id: &str) -> Result<u32, ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::ServerIgnoringCounter {
                client_id: client_id.to_owned(),
                reply,
            })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response.await.map_err(|_| ServerActorError::Unavailable)
    }

    pub async fn time_now_override_seconds(&self) -> Result<Option<f64>, ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::TimeNowOverride { reply })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response.await.map_err(|_| ServerActorError::Unavailable)
    }

    /// Explicit durability barrier for shutdown coordination and tests. Normal
    /// state transitions never wait for this acknowledgement.
    pub async fn flush_persistence(&self) -> Result<(), ServerActorError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::FlushPersistence { reply })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        response
            .await
            .map_err(|_| ServerActorError::Unavailable)?
            .map_err(ServerActorError::from)
    }

    /// Stops the model actor after all earlier commands and persistence effects
    /// have been acknowledged, then joins its task and persistence workers.
    pub async fn shutdown(self) -> Result<(), ServerActorError> {
        self.shutdown_with_timeout(std::time::Duration::from_secs(5))
            .await
    }

    /// One budget covers command queue admission, earlier durability barriers,
    /// acknowledgement and joining. Budgets below 100 ms use that minimum.
    /// A forced outcome is an error even when every worker was safely joined.
    pub async fn shutdown_with_timeout(
        self,
        budget: std::time::Duration,
    ) -> Result<(), ServerActorError> {
        let deadline =
            std::time::Instant::now() + budget.max(std::time::Duration::from_millis(100));
        // Install the terminal deadline before sending to the actor. An earlier
        // Flush may already own both services on a blocking task, and clearing
        // its temporary deadline must never clear this shutdown request.
        for control in &self.persistence_controls {
            control.begin_shutdown(deadline - std::time::Duration::from_millis(100));
        }
        let (reply, response) = oneshot::channel();
        let mut cleanup = tokio::spawn(async move {
            let _ = reply.send(self.shutdown_at(deadline).await);
        });
        match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), &mut cleanup).await
        {
            Ok(result) => {
                result.map_err(|error| ServerActorError::TaskFailed(error.to_string()))?;
                response.await.map_err(|_| ServerActorError::Unavailable)?
            }
            Err(_) => {
                // The task still owns the actor join and all outstanding work.
                // Observation reaps it later; a caller timeout never detaches it.
                persistence_actor::retain_async_cleanup(cleanup);
                Err(ServerActorError::TaskFailed("server actor shutdown deadline exceeded; cleanup ownership retained in the unjoined registry".into()))
            }
        }
    }

    async fn shutdown_at(self, deadline: std::time::Instant) -> Result<(), ServerActorError> {
        let (reply, response) = oneshot::channel();
        let barrier_result = match self
            .commands
            .send(ServerCommand::Shutdown { deadline, reply })
            .await
        {
            Ok(()) => response
                .await
                .map_err(|_| ServerActorError::Unavailable)
                .and_then(|result| result),
            Err(_) => Err(ServerActorError::Unavailable),
        };
        let task_result = match self.join_handle.lock().await.take() {
            Some(join_handle) => join_handle
                .await
                .map_err(|source| ServerActorError::TaskFailed(source.to_string())),
            None => Ok(()),
        };

        match (barrier_result, task_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(barrier), Ok(())) => Err(barrier),
            (Ok(()), Err(task)) => Err(task),
            (Err(barrier), Err(task)) => Err(ServerActorError::ShutdownFailed {
                barrier: barrier.to_string(),
                task: task.to_string(),
            }),
        }
    }
}

async fn run_server_actor(runtime: &mut ServerRuntime, mut commands: Receiver<ServerCommand>) {
    while let Some(command) = commands.recv().await {
        match command {
            #[cfg(test)]
            ServerCommand::PauseForTest { entered, release } => {
                let _ = entered.send(());
                let _ = release.await;
            }
            ServerCommand::HandleLine {
                client_id,
                line,
                peer_ip,
                reply,
            } => {
                let dispatch = runtime.handle_line_fanout_with_transport_actions_for_peer(
                    &client_id,
                    &line,
                    peer_ip.as_deref(),
                );
                let session_exists = runtime.session(&client_id).is_some();
                let _ = reply.send(dispatch.map(|dispatch| (dispatch, session_exists)));
            }
            ServerCommand::Disconnect { client_id, reply } => {
                let _ = reply.send(runtime.handle_transport_disconnect_fanout(&client_id));
            }
            ServerCommand::CollectDispatch { now_seconds, reply } => {
                let _ = reply.send(runtime.collect_dispatch_at(now_seconds));
            }
            ServerCommand::TlsServerConfig { reply } => {
                let _ = reply.send(runtime.tls_server_config());
            }
            ServerCommand::Session { client_id, reply } => {
                let _ = reply.send(runtime.session(&client_id).cloned());
            }
            ServerCommand::ServerIgnoringCounter { client_id, reply } => {
                let _ = reply.send(runtime.server_ignoring_counter(&client_id));
            }
            ServerCommand::TimeNowOverride { reply } => {
                let _ = reply.send(runtime.time_now_override_seconds);
            }
            ServerCommand::FlushPersistence { reply } => {
                let result = persistence_barrier_off_actor(
                    runtime,
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                    false,
                )
                .await;
                let _ = reply.send(result.map_err(|_| {
                    ServerRuntimeError::PersistenceWorkerUnavailable(
                        "durability barrier failed or timed out;",
                    )
                }));
            }
            ServerCommand::Shutdown { deadline, reply } => {
                let result = persistence_barrier_off_actor(runtime, deadline, true).await;
                let _ = reply.send(result);
                break;
            }
        }
    }
}

async fn persistence_barrier_off_actor(
    runtime: &mut ServerRuntime,
    deadline: std::time::Instant,
    shutdown: bool,
) -> Result<(), ServerActorError> {
    let mut rooms = runtime.room_persistence.take();
    let mut stats = runtime.stats_persistence.take();
    let joined = tokio::task::spawn_blocking(move || {
        let grace = deadline.saturating_duration_since(std::time::Instant::now());
        let reserve = grace.min(std::time::Duration::from_millis(100));
        let flush_deadline = if shutdown { deadline - reserve } else { deadline };
        if let Some(service) = &rooms { service.set_deadline(Some(flush_deadline)); }
        if let Some(service) = &stats { service.set_deadline(Some(flush_deadline)); }
        let rooms_durable = rooms.as_ref().is_none_or(|service| service.flush_until(flush_deadline));
        let stats_durable = stats.as_ref().is_none_or(|service| service.flush_until(flush_deadline));
        let timed_out = std::time::Instant::now() >= flush_deadline && !(rooms_durable && stats_durable);
        let mut workers_joined = true;
        if shutdown {
            if let Some(service) = &mut rooms { workers_joined &= service.finish_until(deadline); }
            if let Some(service) = &mut stats { workers_joined &= service.finish_until(deadline); }
        } else {
            if let Some(service) = &rooms { service.set_deadline(None); }
            if let Some(service) = &stats { service.set_deadline(None); }
        }
        let result = if !workers_joined {
            Err(ServerActorError::TaskFailed("persistence shutdown deadline exceeded; worker ownership retained in the unjoined registry".into()))
        } else if timed_out {
            Err(ServerActorError::TaskFailed("persistence durability deadline exceeded; workers stopped without claiming unsaved changes were durable".into()))
        } else if !rooms_durable || !stats_durable {
            Err(ServerActorError::TaskFailed("persistence durability barrier failed; pending changes were not acknowledged".into()))
        } else { Ok(()) };
        (rooms, stats, result)
    }).await.map_err(|error| ServerActorError::TaskFailed(error.to_string()))?;
    let (rooms, stats, result) = joined;
    if !shutdown {
        runtime.room_persistence = rooms;
        runtime.stats_persistence = stats;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_full_queue_timeout_retains_cleanup_until_actor_can_finish() {
        let actor = ServerActorHandle::spawn(ServerRuntime::new());
        let observer = actor.clone();
        let (entered, paused) = oneshot::channel();
        let (release, gate) = oneshot::channel();
        actor
            .commands
            .send(ServerCommand::PauseForTest {
                entered,
                release: gate,
            })
            .await
            .unwrap();
        paused.await.unwrap();
        for _ in 0..SERVER_COMMAND_QUEUE_CAPACITY {
            let (reply, _) = oneshot::channel();
            assert!(
                actor
                    .commands
                    .try_send(ServerCommand::TimeNowOverride { reply })
                    .is_ok()
            );
        }
        let started = std::time::Instant::now();
        let result = actor
            .shutdown_with_timeout(std::time::Duration::from_millis(200))
            .await;
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.to_string().contains("cleanup ownership retained")),
            "{result:?}"
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(persistence_workers_awaiting_join(), 1);
        release.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while persistence_workers_awaiting_join() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(observer.session("no-peer").await.is_err());
    }
}
