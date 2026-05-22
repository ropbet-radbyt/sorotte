use super::*;

#[derive(Debug, Default)]
pub struct ServerApp {
    runtime: ServerRuntime,
}

impl ServerApp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_motd_template(template: impl Into<String>) -> Self {
        Self {
            runtime: ServerRuntime::with_motd_template(template),
        }
    }

    pub fn with_room_password_salt(salt: impl Into<String>) -> Self {
        Self {
            runtime: ServerRuntime::with_room_password_salt(salt),
        }
    }

    pub fn with_persistent_rooms_enabled(enabled: bool) -> Self {
        Self {
            runtime: ServerRuntime::with_persistent_rooms_enabled(enabled),
        }
    }

    pub fn with_stats_db_path(db_path: impl Into<PathBuf>) -> Result<Self, ServerRuntimeError> {
        Ok(Self {
            runtime: ServerRuntime::with_stats_db_path(db_path)?,
        })
    }

    pub fn with_tls_cert_path(path: impl Into<PathBuf>) -> Self {
        Self {
            runtime: ServerRuntime::with_tls_cert_path(path),
        }
    }

    pub fn with_persistent_rooms_db_path(
        db_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        Ok(Self {
            runtime: ServerRuntime::with_persistent_rooms_db_path(db_path)?,
        })
    }

    pub fn with_permanent_rooms_file_path(
        permanent_rooms_file_path: impl Into<PathBuf>,
    ) -> Result<Self, ServerRuntimeError> {
        Ok(Self {
            runtime: ServerRuntime::with_permanent_rooms_file_path(permanent_rooms_file_path)?,
        })
    }

    pub fn runtime_mut(&mut self) -> &mut ServerRuntime {
        &mut self.runtime
    }

    pub fn bootstrap_room(&mut self, room_name: &str) {
        self.runtime.bootstrap_room(room_name);
    }

    pub fn room_is_present(&self, room_name: &str) -> bool {
        self.runtime.room_is_present(room_name)
    }
}
