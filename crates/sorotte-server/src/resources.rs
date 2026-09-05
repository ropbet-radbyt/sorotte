//! Server-owned admission and byte permits. Permits follow sockets and queued
//! work, so disconnect, cancelled futures and panic unwinding release capacity.
use std::{
    collections::BTreeMap,
    io,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerResourceLimits {
    pub active_connections: usize,
    pub unauthenticated_connections: usize,
    pub connections_per_address: usize,
    pub queued_bytes_per_peer: usize,
    pub queued_bytes_total: usize,
}

impl Default for ServerResourceLimits {
    fn default() -> Self {
        Self {
            active_connections: 1024,
            unauthenticated_connections: 128,
            connections_per_address: 128,
            queued_bytes_per_peer: 4 * 1024 * 1024,
            queued_bytes_total: 64 * 1024 * 1024,
        }
    }
}

impl ServerResourceLimits {
    pub fn validate(self) -> io::Result<Self> {
        if self.active_connections == 0
            || self.unauthenticated_connections == 0
            || self.connections_per_address == 0
            || self.queued_bytes_per_peer < 1024
            || self.queued_bytes_total < self.queued_bytes_per_peer
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "server resource limits must be positive; byte limits require 1024 <= per-peer <= total",
            ));
        }
        Ok(self)
    }

    /// Reads operator configuration once at process startup. Invalid settings
    /// are explicit startup errors, never an accidental unlimited fallback.
    pub fn from_environment() -> io::Result<Self> {
        let mut limits = Self::default();
        for (name, target) in [
            (
                "SOROTTE_SERVER_MAX_CONNECTIONS",
                &mut limits.active_connections,
            ),
            (
                "SOROTTE_SERVER_MAX_UNAUTHENTICATED_CONNECTIONS",
                &mut limits.unauthenticated_connections,
            ),
            (
                "SOROTTE_SERVER_MAX_CONNECTIONS_PER_ADDRESS",
                &mut limits.connections_per_address,
            ),
            (
                "SOROTTE_SERVER_MAX_QUEUED_BYTES_PER_PEER",
                &mut limits.queued_bytes_per_peer,
            ),
            (
                "SOROTTE_SERVER_MAX_QUEUED_BYTES_TOTAL",
                &mut limits.queued_bytes_total,
            ),
        ] {
            if let Some(value) = std::env::var_os(name) {
                *target = value
                    .to_str()
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("{name} must be a positive integer"),
                        )
                    })?;
            }
        }
        limits.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ServerResourceSnapshot {
    pub active_connections: usize,
    pub unauthenticated_connections: usize,
    pub address_buckets: usize,
    pub rejected_connections: usize,
    pub queued_bytes: usize,
    pub peak_queued_bytes: usize,
}

#[derive(Debug, Default)]
struct Admission {
    active: usize,
    unauthenticated: usize,
    addresses: BTreeMap<IpAddr, usize>,
    rejected: usize,
}

#[derive(Debug)]
pub(crate) struct NetworkResources {
    pub(crate) limits: ServerResourceLimits,
    admission: Mutex<Admission>,
    bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
}

impl NetworkResources {
    pub(crate) fn new(limits: ServerResourceLimits) -> Arc<Self> {
        Arc::new(Self {
            limits,
            admission: Mutex::new(Admission::default()),
            bytes: AtomicUsize::new(0),
            peak_bytes: AtomicUsize::new(0),
        })
    }
    pub(crate) fn admit(self: &Arc<Self>, address: IpAddr) -> Option<ConnectionPermit> {
        let address = match address {
            IpAddr::V6(address) => address
                .to_ipv4_mapped()
                .map(IpAddr::V4)
                .unwrap_or(IpAddr::V6(address)),
            address => address,
        };
        let mut state = self
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.active >= self.limits.active_connections
            || state.unauthenticated >= self.limits.unauthenticated_connections
            || state.addresses.get(&address).copied().unwrap_or(0)
                >= self.limits.connections_per_address
        {
            state.rejected = state.rejected.saturating_add(1);
            return None;
        }
        state.active += 1;
        state.unauthenticated += 1;
        *state.addresses.entry(address).or_default() += 1;
        Some(ConnectionPermit {
            resources: self.clone(),
            address,
            unauthenticated: true,
        })
    }
    pub(crate) fn snapshot(&self) -> ServerResourceSnapshot {
        let state = self
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        ServerResourceSnapshot {
            active_connections: state.active,
            unauthenticated_connections: state.unauthenticated,
            address_buckets: state.addresses.len(),
            rejected_connections: state.rejected,
            queued_bytes: self.bytes.load(Ordering::Acquire),
            peak_queued_bytes: self.peak_bytes.load(Ordering::Acquire),
        }
    }
    pub(crate) fn peer_budget(self: &Arc<Self>) -> PeerByteBudget {
        PeerByteBudget {
            resources: self.clone(),
            bytes: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ConnectionPermit {
    resources: Arc<NetworkResources>,
    address: IpAddr,
    unauthenticated: bool,
}

impl ConnectionPermit {
    pub(crate) fn authenticated(&mut self) {
        if self.unauthenticated {
            let mut admission = self
                .resources
                .admission
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            admission.unauthenticated -= 1;
            self.unauthenticated = false;
        }
    }
}
impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        let mut state = self
            .resources
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active -= 1;
        if self.unauthenticated {
            state.unauthenticated -= 1;
        }
        if let Some(count) = state.addresses.get_mut(&self.address) {
            *count -= 1;
            if *count == 0 {
                state.addresses.remove(&self.address);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PeerByteBudget {
    resources: Arc<NetworkResources>,
    bytes: Arc<AtomicUsize>,
}

fn reserve(counter: &AtomicUsize, amount: usize, limit: usize) -> Option<usize> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(amount).filter(|next| *next <= limit)
        })
        .ok()
        .map(|previous| previous + amount)
}

impl PeerByteBudget {
    pub(crate) fn reserve(&self, bytes: usize) -> Option<ByteReservation> {
        reserve(
            &self.bytes,
            bytes,
            self.resources.limits.queued_bytes_per_peer,
        )?;
        let Some(total) = reserve(
            &self.resources.bytes,
            bytes,
            self.resources.limits.queued_bytes_total,
        ) else {
            self.bytes.fetch_sub(bytes, Ordering::AcqRel);
            return None;
        };
        self.resources.peak_bytes.fetch_max(total, Ordering::AcqRel);
        Some(ByteReservation {
            budget: self.clone(),
            bytes,
        })
    }
}

#[derive(Debug)]
pub(crate) struct ByteReservation {
    budget: PeerByteBudget,
    bytes: usize,
}

impl ByteReservation {
    pub(crate) fn resize(&mut self, bytes: usize) -> bool {
        match bytes.cmp(&self.bytes) {
            std::cmp::Ordering::Greater => {
                let Some(mut growth) = self.budget.reserve(bytes - self.bytes) else {
                    return false;
                };
                growth.bytes = 0; // Transfer the reservation into this owner.
            }
            std::cmp::Ordering::Equal => return true,
            std::cmp::Ordering::Less => {
                self.budget
                    .bytes
                    .fetch_sub(self.bytes - bytes, Ordering::AcqRel);
                self.budget
                    .resources
                    .bytes
                    .fetch_sub(self.bytes - bytes, Ordering::AcqRel);
            }
        }
        self.bytes = bytes;
        true
    }
}
impl Drop for ByteReservation {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::AcqRel);
        self.budget
            .resources
            .bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_rejects_each_invalid_ceiling_and_preserves_valid_operator_values() {
        let valid = ServerResourceLimits {
            active_connections: 1,
            unauthenticated_connections: 1,
            connections_per_address: 1,
            queued_bytes_per_peer: 1024,
            queued_bytes_total: 1024,
        };
        assert_eq!(valid.validate().unwrap(), valid);
        let larger = ServerResourceLimits {
            active_connections: 10,
            queued_bytes_per_peer: 4096,
            queued_bytes_total: 8192,
            ..valid
        };
        assert_eq!(larger.validate().unwrap(), larger);
        for invalid in [
            ServerResourceLimits {
                active_connections: 0,
                ..valid
            },
            ServerResourceLimits {
                unauthenticated_connections: 0,
                ..valid
            },
            ServerResourceLimits {
                connections_per_address: 0,
                ..valid
            },
            ServerResourceLimits {
                queued_bytes_per_peer: 0,
                ..valid
            },
            ServerResourceLimits {
                queued_bytes_per_peer: 1023,
                ..valid
            },
            ServerResourceLimits {
                queued_bytes_total: 1023,
                ..valid
            },
            ServerResourceLimits {
                queued_bytes_total: 0,
                ..valid
            },
        ] {
            assert_eq!(
                invalid.validate().unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn admission_limits_normalize_addresses_and_release_on_unwind() {
        let resources = NetworkResources::new(ServerResourceLimits {
            active_connections: 3,
            unauthenticated_connections: 2,
            connections_per_address: 2,
            ..ServerResourceLimits::default()
        });
        let ipv4 = "127.0.0.1".parse().unwrap();
        let mut first = resources.admit(ipv4).unwrap();
        let second = resources
            .admit("::ffff:127.0.0.1".parse().unwrap())
            .unwrap();
        assert!(
            resources.admit("::1".parse().unwrap()).is_none(),
            "pre-Hello ceiling"
        );
        first.authenticated();
        first.authenticated();
        assert!(
            resources.admit(ipv4).is_none(),
            "NAT ceiling survives authentication"
        );
        let third = resources.admit("::1".parse().unwrap()).unwrap();
        assert!(resources.admit("127.0.0.2".parse().unwrap()).is_none());
        assert_eq!(resources.snapshot().active_connections, 3);
        assert_eq!(resources.snapshot().address_buckets, 2);
        assert!(
            std::panic::catch_unwind(move || {
                let _owned = (first, second, third);
                panic!("fixture");
            })
            .is_err()
        );
        let snapshot = resources.snapshot();
        assert_eq!(
            (
                snapshot.active_connections,
                snapshot.unauthenticated_connections,
                snapshot.address_buckets
            ),
            (0, 0, 0)
        );
        assert_eq!(snapshot.rejected_connections, 3);
        assert!(resources.admit(ipv4).is_some());
    }

    #[test]
    fn byte_permits_enforce_both_ceilings_and_resize_atomically() {
        let resources = NetworkResources::new(ServerResourceLimits {
            queued_bytes_per_peer: 1024,
            queued_bytes_total: 1536,
            ..ServerResourceLimits::default()
        });
        let first = resources.peer_budget();
        let second = resources.peer_budget();
        let mut a = first.reserve(900).unwrap();
        assert!(first.reserve(125).is_none());
        let b = second.reserve(600).unwrap();
        assert!(
            !a.resize(950),
            "global budget, despite available peer capacity"
        );
        assert_eq!(resources.snapshot().queued_bytes, 1500);
        assert!(a.resize(100));
        assert_eq!(resources.snapshot().queued_bytes, 700);
        assert!(a.resize(936));
        assert!(
            a.resize(936),
            "same-size replacement succeeds even at the global ceiling"
        );
        assert_eq!(resources.snapshot().queued_bytes, 1536);
        assert!(second.reserve(1).is_none());
        assert_eq!(resources.snapshot().peak_queued_bytes, 1536);
        drop((a, b));
        assert_eq!(resources.snapshot().queued_bytes, 0);
        assert!(first.reserve(usize::MAX).is_none());
        assert_eq!(resources.snapshot().queued_bytes, 0);
    }

    #[test]
    fn concurrent_byte_ownership_never_oversubscribes_and_releases_on_panic() {
        let resources = NetworkResources::new(ServerResourceLimits {
            queued_bytes_per_peer: 1024,
            queued_bytes_total: 2048,
            ..ServerResourceLimits::default()
        });
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let resources = resources.clone();
                scope.spawn(move || {
                    let peer = resources.peer_budget();
                    for _ in 0..1000 {
                        let permit = peer.reserve(512);
                        assert!(resources.snapshot().queued_bytes <= 2048);
                        std::hint::black_box(&permit);
                    }
                });
            }
        });
        let peer = resources.peer_budget();
        assert!(
            std::panic::catch_unwind(move || {
                let _permit = peer.reserve(1024).unwrap();
                panic!("fixture");
            })
            .is_err()
        );
        assert_eq!(resources.snapshot().queued_bytes, 0);
        assert!(resources.snapshot().peak_queued_bytes <= 2048);
    }
}
