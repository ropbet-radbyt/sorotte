use super::*;

static CONTROLLED_ROOM_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+(.*):(\w{12})$").expect("controlled room regex is valid"));
static PASSWORD_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Z]{2}-\d{3}-\d{3}").expect("password regex is valid"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RoomPasswordCheckError {
    InvalidPassword,
    NotControlledRoom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomPasswordProvider {
    salt: String,
}

impl Default for RoomPasswordProvider {
    fn default() -> Self {
        Self::new(DEFAULT_CONTROLLED_ROOM_HASH_SALT)
    }
}

impl RoomPasswordProvider {
    pub(crate) fn new(salt: impl Into<String>) -> Self {
        Self { salt: salt.into() }
    }

    pub(crate) fn is_controlled_room_name(&self, room_name: &str) -> bool {
        CONTROLLED_ROOM_REGEX.is_match(room_name)
    }

    pub(crate) fn is_valid_room_password(&self, password: &str) -> bool {
        if password.is_empty() {
            return false;
        }
        PASSWORD_REGEX
            .find(password)
            .is_some_and(|matched| matched.start() == 0)
    }

    pub(crate) fn check(
        &self,
        room_name: &str,
        password: &str,
    ) -> Result<bool, RoomPasswordCheckError> {
        if !self.is_valid_room_password(password) {
            return Err(RoomPasswordCheckError::InvalidPassword);
        }

        let captures = CONTROLLED_ROOM_REGEX
            .captures(room_name)
            .ok_or(RoomPasswordCheckError::NotControlledRoom)?;
        let base_room = captures
            .get(1)
            .expect("controlled room regex always includes base room capture")
            .as_str();
        let expected_hash = captures
            .get(2)
            .expect("controlled room regex always includes hash capture")
            .as_str();
        let computed_hash = self.compute_room_hash(base_room, password);
        Ok(computed_hash == expected_hash)
    }

    pub(crate) fn controlled_room_name_for(&self, room_name: &str, password: &str) -> String {
        format!(
            "+{room_name}:{}",
            self.compute_room_hash(room_name, password)
        )
    }

    pub(crate) fn compute_room_hash(&self, room_name: &str, password: &str) -> String {
        let salt_hash = format!("{:x}", Sha256::digest(self.salt.as_bytes()));
        let provisional_input = format!("{room_name}{salt_hash}");
        let provisional_hash = format!("{:x}", Sha256::digest(provisional_input.as_bytes()));
        let room_hash_input = format!("{provisional_hash}{salt_hash}{password}");
        let room_hash = format!("{:x}", Sha1::digest(room_hash_input.as_bytes()));
        room_hash[..12].to_ascii_uppercase()
    }
}
