//! Plex PIN authentication boundary.

pub use crate::{PlexAuthPollResult, PlexAuthSession};
use crate::{PlexHttpClient, PlexResult};

pub trait PlexAuthTransport {
    fn start_auth(&self) -> PlexResult<PlexAuthSession>;

    fn poll_auth(&self, pin_id: u64) -> PlexResult<PlexAuthPollResult>;
}

#[derive(Clone)]
pub struct PlexAuthService<T> {
    transport: T,
}

impl<T> std::fmt::Debug for PlexAuthService<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlexAuthService")
            .finish_non_exhaustive()
    }
}

impl<T> PlexAuthService<T> {
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }
}

impl<T: PlexAuthTransport> PlexAuthService<T> {
    pub fn start(&self) -> PlexResult<PlexAuthSession> {
        self.transport.start_auth()
    }

    pub fn poll(&self, pin_id: u64) -> PlexResult<PlexAuthPollResult> {
        self.transport.poll_auth(pin_id)
    }
}

impl PlexAuthTransport for PlexHttpClient {
    fn start_auth(&self) -> PlexResult<PlexAuthSession> {
        PlexHttpClient::start_auth(self)
    }

    fn poll_auth(&self, pin_id: u64) -> PlexResult<PlexAuthPollResult> {
        PlexHttpClient::poll_auth(self, pin_id)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use sorotte_secret::SecretValue;

    use super::*;

    #[derive(Clone, Default)]
    struct FakeAuthTransport {
        calls: Rc<RefCell<Vec<String>>>,
    }

    impl std::fmt::Debug for FakeAuthTransport {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("AUTH_TRANSPORT_SECRET_CANARY")
        }
    }

    impl PlexAuthTransport for FakeAuthTransport {
        fn start_auth(&self) -> PlexResult<PlexAuthSession> {
            self.calls.borrow_mut().push("start".to_owned());
            Ok(PlexAuthSession {
                pin_id: 42,
                code: "ABCD".to_owned(),
                auth_url: "https://example.test/auth".to_owned(),
                expires_at: Some("soon".to_owned()),
            })
        }

        fn poll_auth(&self, pin_id: u64) -> PlexResult<PlexAuthPollResult> {
            self.calls.borrow_mut().push(format!("poll:{pin_id}"));
            Ok(PlexAuthPollResult {
                auth_token: Some(SecretValue::from("auth-token")),
                expires_at: Some("later".to_owned()),
            })
        }
    }

    #[test]
    fn auth_service_delegates_start_and_poll_to_owned_transport() {
        let transport = FakeAuthTransport::default();
        let calls = transport.calls.clone();
        let service = PlexAuthService::new(transport);

        let session = service.start().expect("auth start should succeed");
        let poll = service.poll(42).expect("auth poll should succeed");

        assert_eq!(session.pin_id, 42);
        assert_eq!(session.code, "ABCD");
        assert_eq!(
            poll.auth_token.as_ref().map(SecretValue::expose_secret),
            Some("auth-token")
        );
        assert_eq!(calls.borrow().as_slice(), ["start", "poll:42"]);
        assert!(!format!("{service:?}").contains("AUTH_TRANSPORT_SECRET_CANARY"));
    }
}
