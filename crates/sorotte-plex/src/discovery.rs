//! Plex server discovery boundary.

use crate::{PlexError, PlexResult};
pub use crate::{PlexServerConnection, PlexServerConnectionKind, PlexServerDiscoveryTransport};
use sorotte_secret::SecretValue;

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
    pub fn discover(&self, user_token: &SecretValue) -> PlexResult<Vec<PlexServerConnection>> {
        if user_token.is_blank() {
            return Err(PlexError::MissingToken);
        }
        self.transport.discover_servers(user_token)
    }

    pub fn verify(&self, server: &PlexServerConnection) -> PlexResult<()> {
        if server.access_token.is_blank() {
            return Err(PlexError::MissingToken);
        }
        self.transport.verify_server_connection(server)
    }

    pub fn machine_identifier(&self, server_url: &str, token: &SecretValue) -> PlexResult<String> {
        if token.is_blank() {
            return Err(PlexError::MissingToken);
        }
        self.transport.server_machine_identifier(server_url, token)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[derive(Clone, PartialEq, Eq)]
    enum DiscoveryCall {
        Discover(SecretValue),
        Verify(String),
        MachineIdentifier {
            server_url: String,
            token: SecretValue,
        },
    }

    impl std::fmt::Debug for DiscoveryCall {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Discover(token) => formatter.debug_tuple("Discover").field(token).finish(),
                Self::Verify(machine_identifier) => formatter
                    .debug_tuple("Verify")
                    .field(machine_identifier)
                    .finish(),
                Self::MachineIdentifier { token, .. } => formatter
                    .debug_struct("MachineIdentifier")
                    .field("server_url", &sorotte_secret::REDACTED_SECRET)
                    .field("token", token)
                    .finish(),
            }
        }
    }

    #[derive(Clone, Default)]
    struct FakeDiscoveryTransport {
        calls: Rc<RefCell<Vec<DiscoveryCall>>>,
    }

    impl std::fmt::Debug for FakeDiscoveryTransport {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("DISCOVERY_TRANSPORT_SECRET_CANARY")
        }
    }

    impl PlexServerDiscoveryTransport for FakeDiscoveryTransport {
        fn discover_servers(
            &self,
            user_token: &SecretValue,
        ) -> PlexResult<Vec<PlexServerConnection>> {
            self.calls
                .borrow_mut()
                .push(DiscoveryCall::Discover(user_token.clone()));
            Ok(vec![server()])
        }

        fn verify_server_connection(&self, server: &PlexServerConnection) -> PlexResult<()> {
            self.calls
                .borrow_mut()
                .push(DiscoveryCall::Verify(server.machine_identifier.clone()));
            Ok(())
        }

        fn server_machine_identifier(
            &self,
            server_url: &str,
            token: &SecretValue,
        ) -> PlexResult<String> {
            self.calls
                .borrow_mut()
                .push(DiscoveryCall::MachineIdentifier {
                    server_url: server_url.to_owned(),
                    token: token.clone(),
                });
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
        const USER_TOKEN: &str = "DISCOVERY_USER_TOKEN_CANARY";
        const SERVER_TOKEN: &str = "DISCOVERY_SERVER_TOKEN_CANARY";
        const URL_TOKEN: &str = "DISCOVERY_URL_TOKEN_CANARY";
        let transport = FakeDiscoveryTransport::default();
        let calls = transport.calls.clone();
        let service = PlexDiscoveryService::new(transport);
        let user_token = SecretValue::from(USER_TOKEN);
        let server_token = SecretValue::from(SERVER_TOKEN);
        let server_url = format!("https://plex.example.test?X-Plex-Token={URL_TOKEN}");

        let servers = service
            .discover(&user_token)
            .expect("discovery should succeed");
        service
            .verify(&servers[0])
            .expect("verification should succeed");
        let identity = service
            .machine_identifier(&server_url, &server_token)
            .expect("identity should resolve");

        assert_eq!(servers, vec![server()]);
        assert_eq!(identity, "machine-id");
        assert_eq!(
            calls.borrow().as_slice(),
            [
                DiscoveryCall::Discover(user_token),
                DiscoveryCall::Verify("machine-id".to_owned()),
                DiscoveryCall::MachineIdentifier {
                    server_url,
                    token: server_token,
                },
            ]
        );
        let calls_debug = format!("{:?}", calls.borrow());
        assert!(!calls_debug.contains(USER_TOKEN));
        assert!(!calls_debug.contains(SERVER_TOKEN));
        assert!(!calls_debug.contains(URL_TOKEN));
        assert!(calls_debug.contains(sorotte_secret::REDACTED_SECRET));
        assert!(!format!("{service:?}").contains("DISCOVERY_TRANSPORT_SECRET_CANARY"));
    }

    #[test]
    fn http_discovery_rejects_blank_tokens_before_request_construction() {
        let client = crate::PlexHttpClient::with_base_urls(
            "http://127.0.0.1:1",
            "https://app.plex.tv/auth",
            "blank-discovery-token-test",
            "Sorotte",
        )
        .expect("HTTP client should construct");
        let service = PlexDiscoveryService::new(client);

        let error = service
            .discover(&SecretValue::from(" \t\r\n"))
            .expect_err("blank tokens should be rejected without an HTTP attempt");

        assert!(matches!(error, crate::PlexError::MissingToken));
    }
}
