use super::*;
use sorotte_plex::{is_plex_playlist_uri, parse_plex_playlist_uri};

impl ClientSession {
    pub(super) fn list_payload_has_file(file: Option<&Value>) -> bool {
        match file {
            Some(Value::Null) | None => false,
            Some(Value::Object(entries)) => !entries.is_empty(),
            Some(_) => true,
        }
    }

    pub(super) fn list_payload_file_info(
        file: Option<&Value>,
    ) -> (
        bool,
        Option<String>,
        Option<Value>,
        Option<Value>,
        Option<Value>,
    ) {
        match file {
            Some(Value::Null) | None => (false, None, None, None, None),
            Some(Value::Object(entries)) if entries.is_empty() => (false, None, None, None, None),
            Some(value) => (
                Self::list_payload_has_file(Some(value)),
                Self::file_name_from_payload(value),
                Self::file_size_from_payload(value),
                Self::file_duration_from_payload(value),
                Self::media_match_signature_from_payload(value),
            ),
        }
    }

    pub(super) fn file_name_from_payload(file: &Value) -> Option<String> {
        match file {
            Value::Object(entries) => entries
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Value::String(name) => Some(name.to_owned()),
            _ => None,
        }
    }

    pub(super) fn file_size_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries.get("size").cloned(),
            _ => None,
        }
    }

    pub(super) fn file_duration_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries.get("duration").cloned(),
            _ => None,
        }
    }

    pub(super) fn media_match_signature_from_payload(file: &Value) -> Option<Value> {
        match file {
            Value::Object(entries) => entries
                .get(MEDIA_MATCH_FILE_PAYLOAD_KEY)
                .filter(|value| matches!(value, Value::Object(_)))
                .cloned(),
            _ => None,
        }
    }

    pub(super) fn file_metadata_from_payload(
        file: &Value,
    ) -> (Option<String>, Option<Value>, Option<Value>, Option<Value>) {
        (
            Self::file_name_from_payload(file),
            Self::file_size_from_payload(file),
            Self::file_duration_from_payload(file),
            Self::media_match_signature_from_payload(file),
        )
    }

    pub(super) fn file_difference_summary_for_users(
        current_user: &ClientUserView,
        other_user: &ClientUserView,
        session: &ClientSession,
    ) -> Option<FileDifferenceSummary> {
        if !current_user.has_file || !other_user.has_file {
            return None;
        }

        let filename = match (&current_user.file_name, &other_user.file_name) {
            (Some(current_name), Some(other_name)) => {
                !Self::same_filename_legacy_like(current_name, other_name)
            }
            (None, None) => false,
            _ => true,
        };

        let filesize = match (&current_user.file_size, &other_user.file_size) {
            (Some(current_size), Some(other_size)) => {
                !Self::same_filesize_legacy_like(current_size, other_size)
            }
            (None, None) => false,
            _ => true,
        };

        let fileduration = match (&current_user.file_duration, &other_user.file_duration) {
            (Some(current_duration), Some(other_duration)) => {
                match (current_duration.as_f64(), other_duration.as_f64()) {
                    (Some(current_duration), Some(other_duration)) => !session
                        .same_fileduration_with_readiness_autoplay_config(
                            current_duration,
                            other_duration,
                        ),
                    _ => true,
                }
            }
            (None, None) => false,
            _ => true,
        };

        Some(FileDifferenceSummary {
            filename,
            filesize,
            fileduration,
        })
    }

    pub fn current_user_file_name(&self) -> Option<&str> {
        self.username
            .as_deref()
            .and_then(|username| self.user_file_name(username))
    }

    pub(super) fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }

    pub(super) fn percent_decode_lossy(input: &str) -> String {
        let bytes = input.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;

        while index < bytes.len() {
            if bytes[index] == b'%'
                && index + 2 < bytes.len()
                && let (Some(high), Some(low)) = (
                    Self::hex_value(bytes[index + 1]),
                    Self::hex_value(bytes[index + 2]),
                )
            {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
            decoded.push(bytes[index]);
            index += 1;
        }

        String::from_utf8_lossy(&decoded).into_owned()
    }

    pub(super) fn strip_filename_for_compare(filename: &str, strip_url: bool) -> String {
        let decoded_filename = Self::percent_decode_lossy(filename);
        let normalized_name = if strip_url {
            let last_segment = decoded_filename
                .rsplit('/')
                .next()
                .unwrap_or(&decoded_filename);
            Self::percent_decode_lossy(last_segment)
        } else {
            decoded_filename
        };
        normalized_name
            .chars()
            .filter(|ch| {
                !matches!(
                    ch,
                    '-' | '~' | '_' | '.' | '[' | ']' | '(' | ')' | ':' | ' '
                )
            })
            .collect()
    }

    pub(super) fn same_hashed_legacy_like(
        left_raw: &str,
        left_hash: &str,
        right_raw: &str,
        right_hash: &str,
    ) -> bool {
        left_raw.to_lowercase() == right_raw.to_lowercase()
            || left_raw == right_raw
            || left_raw == right_hash
            || left_hash == right_raw
            || left_hash == right_hash
    }

    pub(super) fn is_web_url(filename: &str) -> bool {
        let filename = filename.trim_start();
        filename
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            || filename
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    }

    pub(super) fn is_plex_uri(filename: &str) -> bool {
        is_plex_playlist_uri(filename)
    }

    pub(super) fn is_url(filename: &str) -> bool {
        Self::is_web_url(filename)
    }

    pub(super) fn hash_filename_for_compare(filename: &str) -> String {
        format!("{:x}", Sha256::digest(filename.as_bytes()))[..12].to_owned()
    }

    pub(super) fn hash_filesize_for_compare(filesize_raw: &str) -> String {
        format!("{:x}", Sha256::digest(filesize_raw.as_bytes()))[..12].to_owned()
    }

    pub(super) fn filename_with_privacy_mode_legacy_like(
        file_name: &Value,
        privacy_mode: PrivacyMode,
    ) -> Option<String> {
        match privacy_mode {
            PrivacyMode::SendRaw => file_name.as_str().map(str::to_owned),
            PrivacyMode::SendHashed => {
                let raw_name = file_name.as_str()?;
                let strip_url = Self::is_url(raw_name);
                let stripped_name = Self::strip_filename_for_compare(raw_name, strip_url);
                Some(Self::hash_filename_for_compare(&stripped_name))
            }
            PrivacyMode::DoNotSend => Some(PRIVACY_HIDDEN_FILENAME.to_owned()),
        }
    }

    pub(super) fn filesize_raw_for_privacy(size: &Value) -> String {
        match size {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.clone(),
            Value::Bool(boolean) => boolean.to_string(),
            Value::Null => "None".to_owned(),
            Value::Array(_) | Value::Object(_) => size.to_string(),
        }
    }

    pub(super) fn filesize_with_privacy_mode_legacy_like(
        size: &Value,
        privacy_mode: PrivacyMode,
    ) -> Option<Value> {
        match privacy_mode {
            PrivacyMode::SendRaw => Some(size.clone()),
            PrivacyMode::SendHashed => {
                let raw_size = Self::filesize_raw_for_privacy(size);
                Some(Value::String(Self::hash_filesize_for_compare(&raw_size)))
            }
            PrivacyMode::DoNotSend => Some(Value::from(0)),
        }
    }

    pub(super) fn filesize_is_zero_legacy_like(filesize: &Value) -> bool {
        match filesize {
            Value::Number(number) => {
                if let Some(signed) = number.as_i64() {
                    signed == 0
                } else if let Some(unsigned) = number.as_u64() {
                    unsigned == 0
                } else {
                    number.as_f64().is_some_and(|float| float == 0.0)
                }
            }
            _ => false,
        }
    }

    pub(super) fn filesize_raw_for_compare(filesize: &Value) -> Option<String> {
        match filesize {
            Value::Number(number) => Some(number.to_string()),
            Value::String(text) => Some(text.clone()),
            _ => None,
        }
    }

    pub(super) fn same_filesize_legacy_like(left: &Value, right: &Value) -> bool {
        if Self::filesize_is_zero_legacy_like(left) || Self::filesize_is_zero_legacy_like(right) {
            return true;
        }

        let Some(left_raw) = Self::filesize_raw_for_compare(left) else {
            return false;
        };
        let Some(right_raw) = Self::filesize_raw_for_compare(right) else {
            return false;
        };

        let left_hash = Self::hash_filesize_for_compare(&left_raw);
        let right_hash = Self::hash_filesize_for_compare(&right_raw);
        Self::same_hashed_legacy_like(&left_raw, &left_hash, &right_raw, &right_hash)
    }

    pub(super) fn round_half_to_even(value: f64) -> f64 {
        let floor = value.floor();
        let fraction = value - floor;

        if fraction + ROUND_HALF_EPSILON < 0.5 {
            return floor;
        }
        if fraction - ROUND_HALF_EPSILON > 0.5 {
            return floor + 1.0;
        }

        if floor.rem_euclid(2.0) == 0.0 {
            floor
        } else {
            floor + 1.0
        }
    }

    pub(super) fn same_fileduration_legacy_like(
        left: f64,
        right: f64,
        show_duration_notification: bool,
        different_duration_threshold: f64,
    ) -> bool {
        if !show_duration_notification {
            return true;
        }

        (Self::round_half_to_even(left) - Self::round_half_to_even(right)).abs()
            < different_duration_threshold
    }

    fn filename_compare_stem(value: &str) -> Option<String> {
        let basename = value
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_owned();
        basename
            .rsplit_once('.')
            .map(|(stem, _)| stem.trim().to_owned())
            .filter(|stem| !stem.is_empty())
    }

    fn filename_compare_candidates(value: &str) -> Vec<String> {
        let mut candidates = Vec::new();
        if Self::is_plex_uri(value) {
            if let Ok(uri) = parse_plex_playlist_uri(value) {
                if let Some(file_name) = uri.file_name {
                    candidates.push(file_name.clone());
                    if let Some(stem) = Self::filename_compare_stem(&file_name) {
                        candidates.push(stem);
                    }
                }
                if let Some(title) = uri.title {
                    candidates.push(title);
                }
            }
        } else {
            candidates.push(value.to_owned());
            if let Some(stem) = Self::filename_compare_stem(value) {
                candidates.push(stem);
            }
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }

    fn same_filename_without_plex_uri_hints(left: &str, right: &str) -> bool {
        let strip_url = Self::is_url(left) ^ Self::is_url(right);
        let left_stripped = Self::strip_filename_for_compare(left, strip_url);
        let right_stripped = Self::strip_filename_for_compare(right, strip_url);
        let left_hash = Self::hash_filename_for_compare(&left_stripped);
        let right_hash = Self::hash_filename_for_compare(&right_stripped);
        Self::same_hashed_legacy_like(&left_stripped, &left_hash, &right_stripped, &right_hash)
    }

    pub(super) fn same_filename_legacy_like(left: &str, right: &str) -> bool {
        if left == PRIVACY_HIDDEN_FILENAME || right == PRIVACY_HIDDEN_FILENAME {
            return true;
        }
        if Self::is_plex_uri(left) || Self::is_plex_uri(right) {
            let left_candidates = Self::filename_compare_candidates(left);
            let right_candidates = Self::filename_compare_candidates(right);
            return left_candidates.iter().any(|left_candidate| {
                right_candidates.iter().any(|right_candidate| {
                    Self::same_filename_without_plex_uri_hints(left_candidate, right_candidate)
                })
            });
        }
        Self::same_filename_without_plex_uri_hints(left, right)
    }

    pub(super) fn file_payload_from_user_view(user_view: &ClientUserView) -> Option<Value> {
        if !user_view.has_file {
            return None;
        }

        let mut payload = Map::new();
        if let Some(file_name) = user_view.file_name.as_ref() {
            payload.insert("name".to_owned(), Value::String(file_name.clone()));
        }
        if let Some(file_size) = user_view.file_size.as_ref() {
            payload.insert("size".to_owned(), file_size.clone());
        }
        if let Some(file_duration) = user_view.file_duration.as_ref() {
            payload.insert("duration".to_owned(), file_duration.clone());
        }
        if let Some(media_match_signature) = user_view.media_match_signature.as_ref() {
            payload.insert(
                MEDIA_MATCH_FILE_PAYLOAD_KEY.to_owned(),
                media_match_signature.clone(),
            );
        }

        Some(Value::Object(payload))
    }

    pub(super) fn is_music_file_name(file_name: &str) -> bool {
        let lower_name = file_name.to_ascii_lowercase();
        MUSIC_FORMATS
            .iter()
            .any(|music_format| lower_name.ends_with(music_format))
    }
}
