use super::*;
use tokio::sync::oneshot;

const SERVER_COMMAND_QUEUE_CAPACITY: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ServerActorError {
    #[error(transparent)]
    Runtime(#[from] ServerRuntimeError),
    #[error("server actor is unavailable")]
    Unavailable,
}

#[derive(Debug)]
enum ServerCommand {
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
    TimeNowOverride {
        reply: oneshot::Sender<Option<f64>>,
    },
    FlushPersistence {
        reply: oneshot::Sender<Result<(), ServerRuntimeError>>,
    },
    Shutdown {
        reply: oneshot::Sender<Result<(), ServerRuntimeError>>,
    },
}

/// Bounded, cloneable ingress for the single task that owns the server model.
#[derive(Debug, Clone)]
pub struct ServerActorHandle {
    commands: Sender<ServerCommand>,
    persistence_events: broadcast::Sender<ServerPersistenceEvent>,
    persistence_degraded_worker_count: Arc<AtomicUsize>,
    join_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl ServerActorHandle {
    pub fn spawn(mut runtime: ServerRuntime) -> Self {
        let (commands, receiver) = channel(SERVER_COMMAND_QUEUE_CAPACITY);
        let persistence_events = runtime.persistence_events.clone();
        let persistence_degraded_worker_count = runtime.persistence_degraded_worker_count.clone();
        let join_handle = tokio::spawn(async move {
            run_server_actor(&mut runtime, receiver).await;
        });
        Self {
            commands,
            persistence_events,
            persistence_degraded_worker_count,
            join_handle: Arc::new(Mutex::new(Some(join_handle))),
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
        let (reply, response) = oneshot::channel();
        self.commands
            .send(ServerCommand::Shutdown { reply })
            .await
            .map_err(|_| ServerActorError::Unavailable)?;
        let flush_result = response
            .await
            .map_err(|_| ServerActorError::Unavailable)?
            .map_err(ServerActorError::from);
        if let Some(join_handle) = self.join_handle.lock().await.take() {
            join_handle
                .await
                .map_err(|_| ServerActorError::Unavailable)?;
        }
        flush_result
    }
}

async fn run_server_actor(runtime: &mut ServerRuntime, mut commands: Receiver<ServerCommand>) {
    while let Some(command) = commands.recv().await {
        match command {
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
            ServerCommand::TimeNowOverride { reply } => {
                let _ = reply.send(runtime.time_now_override_seconds);
            }
            ServerCommand::FlushPersistence { reply } => {
                let _ = reply.send(runtime.flush_persistence());
            }
            ServerCommand::Shutdown { reply } => {
                let result = runtime.flush_persistence();
                let _ = reply.send(result);
                break;
            }
        }
    }
}
