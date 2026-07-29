use super::*;

static CONTROLLED_ROOM_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\+(.*):(\w{12})$").expect("controlled room regex is valid"));
static PASSWORD_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Z]{2}-\d{3}-\d{3}$").expect("password regex is valid"));
const GENERATED_SERVER_SALT_LENGTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RoomPasswordCheckError {
    InvalidPassword,
    NotControlledRoom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoomPasswordProvider {
    salt: SecretValue,
}

impl Default for RoomPasswordProvider {
    fn default() -> Self {
        Self::new(DEFAULT_CONTROLLED_ROOM_HASH_SALT)
    }
}

impl RoomPasswordProvider {
    pub(crate) fn new(salt: impl Into<SecretValue>) -> Self {
        Self { salt: salt.into() }
    }

    pub(crate) fn is_controlled_room_name(&self, room_name: &str) -> bool {
        CONTROLLED_ROOM_REGEX.is_match(room_name)
    }

    pub(crate) fn is_valid_room_password(&self, password: &str) -> bool {
        PASSWORD_REGEX.is_match(password)
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
        let (Some(base_room), Some(expected_hash)) = (captures.get(1), captures.get(2)) else {
            return Err(RoomPasswordCheckError::NotControlledRoom);
        };
        let base_room = base_room.as_str();
        let expected_hash = expected_hash.as_str();
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
        let salt_hash = lowercase_hex(Sha256::digest(self.salt.expose_secret().as_bytes()));
        let provisional_input = format!("{room_name}{salt_hash}");
        let provisional_hash = lowercase_hex(Sha256::digest(provisional_input.as_bytes()));
        let room_hash_input = format!("{provisional_hash}{salt_hash}{password}");
        let room_hash = lowercase_hex(Sha1::digest(room_hash_input.as_bytes()));
        room_hash[..12].to_ascii_uppercase()
    }
}

pub(crate) fn generate_server_salt_legacy_compatible() -> String {
    let mut bytes = [0_u8; GENERATED_SERVER_SALT_LENGTH];
    getrandom::fill(&mut bytes).expect("operating system random source should be available");
    bytes.iter().copied().map(legacy_salt_character).collect()
}

fn legacy_salt_character(byte: u8) -> char {
    char::from(b'A' + (byte % 26))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_room_name_recognizes_only_legacy_grammar() {
        let provider = RoomPasswordProvider::default();

        assert!(provider.is_controlled_room_name("+room1:CB39A19549E8"));
        for ordinary_or_malformed in [
            "room1",
            "room1:CB39A19549E8",
            "+room1:CB39A19549E",
            "+room1:CB39A19549E88",
            "+room1:CB39A195-9E8",
        ] {
            assert!(
                !provider.is_controlled_room_name(ordinary_or_malformed),
                "{ordinary_or_malformed:?} must not be recognized as a controlled room"
            );
        }
        assert_eq!(
            provider.check("room1", "AB-123-456"),
            Err(RoomPasswordCheckError::NotControlledRoom)
        );
    }

    #[test]
    fn room_password_provider_matches_legacy_sha_hash_output() {
        let provider = RoomPasswordProvider::default();
        let controlled_room_name = provider.controlled_room_name_for("room1", "AB-123-456");
        assert_eq!(controlled_room_name, "+room1:CB39A19549E8");
        assert_eq!(
            provider.check(&controlled_room_name, "AB-123-456"),
            Ok(true)
        );
        assert_eq!(
            provider.check(&controlled_room_name, "AB-123-457"),
            Ok(false)
        );
    }

    #[test]
    fn controlled_room_password_accepts_exact_legacy_format() {
        let provider = RoomPasswordProvider::default();
        assert!(provider.is_valid_room_password("AB-123-456"));
        assert_eq!(
            provider.check("+room1:CB39A19549E8", "AB-123-456"),
            Ok(true)
        );
    }

    #[test]
    fn controlled_room_password_rejects_trailing_characters() {
        let provider = RoomPasswordProvider::default();
        assert!(!provider.is_valid_room_password("AB-123-4567"));
        assert!(!provider.is_valid_room_password("AB-123-456-extra"));
        assert!(!provider.is_valid_room_password("ab-123-456"));
        assert_eq!(
            provider.check("+room1:CB39A19549E8", "AB-123-4567"),
            Err(RoomPasswordCheckError::InvalidPassword)
        );
        assert_eq!(
            provider.check("+room1:CB39A19549E8", "bad-password"),
            Err(RoomPasswordCheckError::InvalidPassword)
        );
    }

    #[test]
    fn room_password_provider_salt_changes_controlled_room_hashes() {
        let default_provider = RoomPasswordProvider::default();
        let custom_provider = RoomPasswordProvider::new("custom-salt");
        let password = "AB-123-456";
        let default_room_name = default_provider.controlled_room_name_for("room1", password);
        let custom_room_name = custom_provider.controlled_room_name_for("room1", password);
        assert_ne!(custom_room_name, default_room_name);
        assert_eq!(custom_provider.check(&custom_room_name, password), Ok(true));
        assert_eq!(
            default_provider.check(&custom_room_name, password),
            Ok(false)
        );
    }

    #[test]
    fn legacy_salt_character_wraps_the_complete_uppercase_alphabet() {
        assert_eq!(legacy_salt_character(0), 'A');
        assert_eq!(legacy_salt_character(25), 'Z');
        assert_eq!(legacy_salt_character(26), 'A');
        assert_eq!(legacy_salt_character(u8::MAX), 'V');
    }

    #[test]
    fn generated_server_salt_matches_legacy_shape() {
        let salt = generate_server_salt_legacy_compatible();

        assert_eq!(salt.len(), 10);
        assert!(salt.chars().all(|character| character.is_ascii_uppercase()));
    }
}
