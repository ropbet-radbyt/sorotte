//! Plex server discovery boundary.

use crate::PlexResult;
pub use crate::{PlexServerConnection, PlexServerConnectionKind, PlexServerDiscoveryTransport};

#[derive(Clone)]
pub struct PlexDiscoveryService<T> {
    transport: T,
}

impl<T> std::fmt::Debug for PlexDiscoveryService<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlexDiscoveryService")
            .finish_non_exhaustive()
    }
}

impl<T> PlexDiscoveryService<T> {
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: PlexServerDiscoveryTransport> PlexDiscoveryService<T> {
    pub fn discover(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>> {
        self.transport.discover_servers(user_token)
    }

    pub fn verify(&self, server: &PlexServerConnection) -> PlexResult<()> {
        self.transport.verify_server_connection(server)
    }

    pub fn machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String> {
        self.transport.server_machine_identifier(server_url, token)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[derive(Clone, Default)]
    struct FakeDiscoveryTransport {
        calls: Rc<RefCell<Vec<String>>>,
    }

    impl std::fmt::Debug for FakeDiscoveryTransport {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("DISCOVERY_TRANSPORT_SECRET_CANARY")
        }
    }

    impl PlexServerDiscoveryTransport for FakeDiscoveryTransport {
        fn discover_servers(&self, user_token: &str) -> PlexResult<Vec<PlexServerConnection>> {
            self.calls
                .borrow_mut()
                .push(format!("discover:{user_token}"));
            Ok(vec![server()])
        }

        fn verify_server_connection(&self, server: &PlexServerConnection) -> PlexResult<()> {
            self.calls
                .borrow_mut()
                .push(format!("verify:{}", server.machine_identifier));
            Ok(())
        }

        fn server_machine_identifier(&self, server_url: &str, token: &str) -> PlexResult<String> {
            self.calls
                .borrow_mut()
                .push(format!("identity:{server_url}:{token}"));
            Ok("machine-id".to_owned())
        }
    }

    fn server() -> PlexServerConnection {
        PlexServerConnection {
            name: "Living Room".to_owned(),
            machine_identifier: "machine-id".to_owned(),
            uri: "https://plex.example.test".to_owned(),
            access_token: "server-token".into(),
            owned: true,
            has_local_connection: false,
            connection_kind: PlexServerConnectionKind::Remote,
        }
    }

    #[test]
    fn discovery_service_delegates_discover_verify_and_identity_to_owned_transport() {
        let transport = FakeDiscoveryTransport::default();
        let calls = transport.calls.clone();
        let service = PlexDiscoveryService::new(transport);

        let servers = service
            .discover("user-token")
            .expect("discovery should succeed");
        service
            .verify(&servers[0])
            .expect("verification should succeed");
        let identity = service
            .machine_identifier("https://plex.example.test", "server-token")
            .expect("identity should resolve");

        assert_eq!(servers, vec![server()]);
        assert_eq!(identity, "machine-id");
        assert_eq!(
            calls.borrow().as_slice(),
            [
                "discover:user-token",
                "verify:machine-id",
                "identity:https://plex.example.test:server-token",
            ]
        );
        assert!(!format!("{service:?}").contains("DISCOVERY_TRANSPORT_SECRET_CANARY"));
    }
}
