use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(crate) struct RoomRoster {
    rooms: BTreeMap<String, BTreeMap<String, Option<bool>>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RoomRosterError {
    #[error("room not found: {0}")]
    RoomMissing(String),
    #[error("user not found: {0}")]
    UserMissing(String),
}

impl RoomRoster {
    pub(crate) fn join_room(&mut self, username: &str, room_name: &str) {
        self.join_room_with_ready(username, room_name, None);
    }

    pub(crate) fn join_room_with_ready(
        &mut self,
        username: &str,
        room_name: &str,
        ready: Option<bool>,
    ) {
        self.rooms
            .entry(room_name.to_owned())
            .or_default()
            .insert(username.to_owned(), ready);
    }

    pub(crate) fn leave_room(
        &mut self,
        username: &str,
        room_name: &str,
    ) -> Result<(), RoomRosterError> {
        let users = self
            .rooms
            .get_mut(room_name)
            .ok_or_else(|| RoomRosterError::RoomMissing(room_name.to_owned()))?;
        if users.remove(username).is_none() {
            return Err(RoomRosterError::UserMissing(username.to_owned()));
        }
        if users.is_empty() {
            self.rooms.remove(room_name);
        }
        Ok(())
    }

    pub(crate) fn set_ready(
        &mut self,
        username: &str,
        room_name: &str,
        ready: bool,
    ) -> Result<(), RoomRosterError> {
        let users = self
            .rooms
            .get_mut(room_name)
            .ok_or_else(|| RoomRosterError::RoomMissing(room_name.to_owned()))?;
        let user_ready = users
            .get_mut(username)
            .ok_or_else(|| RoomRosterError::UserMissing(username.to_owned()))?;
        *user_ready = Some(ready);
        Ok(())
    }

    pub(crate) fn contains_room(&self, room_name: &str) -> bool {
        self.rooms.contains_key(room_name)
    }

    pub(crate) fn user_ready(&self, username: &str, room_name: &str) -> Option<bool> {
        self.rooms.get(room_name)?.get(username).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::{RoomRoster, RoomRosterError};

    #[test]
    fn readiness_is_scoped_to_each_room_and_preserved_on_join() {
        let mut roster = RoomRoster::default();
        roster.join_room("alice", "room1");
        roster.join_room_with_ready("bob", "room1", Some(false));
        roster.join_room_with_ready("alice", "room2", Some(false));

        assert_eq!(roster.user_ready("alice", "room1"), None);
        roster.set_ready("alice", "room1", true).unwrap();
        assert_eq!(roster.user_ready("alice", "room1"), Some(true));
        assert_eq!(roster.user_ready("bob", "room1"), Some(false));
        assert_eq!(roster.user_ready("alice", "room2"), Some(false));
        assert_eq!(roster.user_ready("missing", "room1"), None);
        assert_eq!(roster.user_ready("alice", "missing"), None);

        roster.join_room_with_ready("alice", "room1", Some(false));
        assert_eq!(roster.user_ready("alice", "room1"), Some(false));
        roster.join_room("alice", "room1");
        assert_eq!(roster.user_ready("alice", "room1"), None);
    }

    #[test]
    fn last_leave_removes_the_room_even_when_readiness_is_unknown() {
        let mut roster = RoomRoster::default();
        roster.join_room("alice", "room1");
        roster.join_room("bob", "room1");

        roster.leave_room("bob", "room1").unwrap();
        assert!(roster.contains_room("room1"));
        roster.leave_room("alice", "room1").unwrap();
        assert!(!roster.contains_room("room1"));
    }

    #[test]
    fn invalid_mutations_report_the_missing_room_or_user_without_changing_readiness() {
        let mut roster = RoomRoster::default();
        roster.join_room_with_ready("alice", "room1", Some(true));

        assert_eq!(
            roster.leave_room("bob", "room1"),
            Err(RoomRosterError::UserMissing("bob".to_owned())),
        );
        assert_eq!(
            roster.set_ready("bob", "room1", false),
            Err(RoomRosterError::UserMissing("bob".to_owned())),
        );
        assert_eq!(
            roster.leave_room("alice", "missing"),
            Err(RoomRosterError::RoomMissing("missing".to_owned())),
        );
        assert_eq!(
            roster.set_ready("alice", "missing", false),
            Err(RoomRosterError::RoomMissing("missing".to_owned())),
        );
        assert_eq!(roster.user_ready("alice", "room1"), Some(true));
        assert!(!roster.contains_room("missing"));
    }
}
