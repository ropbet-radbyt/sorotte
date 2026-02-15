use std::collections::BTreeMap;

pub type Username = String;
pub type RoomName = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserState {
    pub username: Username,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomState {
    pub name: RoomName,
    pub users: BTreeMap<Username, UserState>,
}

#[derive(Debug, Default)]
pub struct SyncDomain {
    rooms: BTreeMap<RoomName, RoomState>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("room not found: {0}")]
    RoomMissing(RoomName),
    #[error("user not found: {0}")]
    UserMissing(Username),
}

impl SyncDomain {
    pub fn join_room(&mut self, username: &str, room_name: &str) {
        let room = self
            .rooms
            .entry(room_name.to_owned())
            .or_insert_with(|| RoomState {
                name: room_name.to_owned(),
                users: BTreeMap::new(),
            });

        room.users.insert(
            username.to_owned(),
            UserState {
                username: username.to_owned(),
                ready: false,
            },
        );
    }

    pub fn leave_room(&mut self, username: &str, room_name: &str) -> Result<(), DomainError> {
        let room = self
            .rooms
            .get_mut(room_name)
            .ok_or_else(|| DomainError::RoomMissing(room_name.to_owned()))?;
        let removed = room.users.remove(username);
        if removed.is_none() {
            return Err(DomainError::UserMissing(username.to_owned()));
        }
        if room.users.is_empty() {
            self.rooms.remove(room_name);
        }
        Ok(())
    }

    pub fn set_ready(
        &mut self,
        username: &str,
        room_name: &str,
        ready: bool,
    ) -> Result<(), DomainError> {
        let room = self
            .rooms
            .get_mut(room_name)
            .ok_or_else(|| DomainError::RoomMissing(room_name.to_owned()))?;
        let user = room
            .users
            .get_mut(username)
            .ok_or_else(|| DomainError::UserMissing(username.to_owned()))?;
        user.ready = ready;
        Ok(())
    }

    pub fn users_in_room(&self, room_name: &str) -> Option<Vec<&UserState>> {
        self.rooms
            .get(room_name)
            .map(|room| room.users.values().collect::<Vec<_>>())
    }
}

#[cfg(test)]
mod tests {
    use super::{DomainError, SyncDomain};

    #[test]
    fn join_and_ready_flow() {
        let mut domain = SyncDomain::default();
        domain.join_room("alice", "room1");
        domain.join_room("bob", "room1");

        domain
            .set_ready("alice", "room1", true)
            .expect("room and user should exist");

        let users = domain
            .users_in_room("room1")
            .expect("room should exist after joins");
        let ready_count = users.iter().filter(|user| user.ready).count();
        assert_eq!(ready_count, 1);
        assert_eq!(users.len(), 2);
    }

    #[test]
    fn leaving_unknown_user_returns_error() {
        let mut domain = SyncDomain::default();
        domain.join_room("alice", "room1");
        let error = domain
            .leave_room("bob", "room1")
            .expect_err("unknown user should error");
        assert_eq!(error, DomainError::UserMissing("bob".to_owned()));
    }
}
