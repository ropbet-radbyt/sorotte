use std::fmt;

pub const REDACTED_SECRET: &str = "<redacted>";

/// Returns whether a structured field name is expected to carry credentials
/// or another value that must not cross a diagnostic formatting boundary.
///
/// Wire formats use several naming conventions, so classification ignores
/// ASCII punctuation and case. Suffix matching is intentionally limited to
/// names whose suffix itself denotes a sensitive value; generic `key` fields
/// are not classified.
pub fn key_is_sensitive(key: &str) -> bool {
    let canonical = canonical_sensitive_key(key.as_bytes());
    ["logicalmediaid", "operationid", "requestid"]
        .into_iter()
        .any(|privacy_id| canonical.contains(privacy_id))
        || credential_key_is_sensitive(key)
}

fn credential_key_is_sensitive(key: &str) -> bool {
    let canonical = canonical_sensitive_key(key.as_bytes());
    matches!(
        canonical.as_str(),
        "authorization"
            | "cookie"
            | "cookies"
            | "credentials"
            | "headers"
            | "httpheaders"
            | "httpheaderfields"
            | "proxyauthorization"
    ) || [
        "password",
        "passwd",
        "passphrase",
        "token",
        "secret",
        "credential",
        "credentials",
        "apikey",
        "authorization",
        "cookie",
        "cookies",
        "headers",
    ]
    .into_iter()
    .any(|suffix| canonical.ends_with(suffix))
        || sensitive_key_words(key).any(|word| {
            matches!(
                word.as_str(),
                "password"
                    | "passwd"
                    | "passphrase"
                    | "token"
                    | "secret"
                    | "credential"
                    | "credentials"
                    | "authorization"
                    | "cookie"
                    | "cookies"
                    | "headers"
            )
        })
}

/// Detects credential-bearing field syntax in an untrusted diagnostic.
///
/// Detection recognizes the small ASCII escape vocabulary used by JSON and
/// URL diagnostics, then examines the identifier immediately before `=` or
/// `:`. It does not decode or return the diagnostic itself. The exact prose
/// phrase `token: EOF` remains an ordinary parser diagnostic unless the value
/// is quoted or uses an authentication scheme.
pub fn text_may_contain_credentials(value: &str) -> bool {
    let projected = diagnostic_ascii_projection(value);
    projected
        .iter()
        .enumerate()
        .filter(|(_, byte)| matches!(byte, b'=' | b':'))
        .any(|(delimiter_index, delimiter)| {
            let Some((key_start, key)) = credential_key_before(&projected, delimiter_index) else {
                return false;
            };
            if !credential_key_is_sensitive(&key) {
                return false;
            }
            if *delimiter == b'=' || canonical_sensitive_key(key.as_bytes()) != "token" {
                return true;
            }
            token_colon_has_credential_shape(&projected, key_start, delimiter_index + 1)
        })
}

fn canonical_sensitive_key(bytes: &[u8]) -> String {
    bytes
        .iter()
        .copied()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

fn sensitive_key_words(key: &str) -> impl Iterator<Item = String> + '_ {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_was_lowercase_or_digit = false;
    for character in key.chars() {
        if !character.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_was_lowercase_or_digit = false;
            continue;
        }
        if character.is_ascii_uppercase() && previous_was_lowercase_or_digit && !current.is_empty()
        {
            words.push(std::mem::take(&mut current));
        }
        current.push(character.to_ascii_lowercase());
        previous_was_lowercase_or_digit =
            character.is_ascii_lowercase() || character.is_ascii_digit();
    }
    if !current.is_empty() {
        words.push(current);
    }
    words.into_iter()
}

fn diagnostic_ascii_projection(value: &str) -> Vec<u8> {
    let bytes = value.as_bytes();
    let mut projected = Vec::with_capacity(bytes.len());
    let mut remaining = bytes;
    while !remaining.is_empty() {
        match remaining {
            [b'%', high, low, tail @ ..] => {
                if let Some(decoded) = decode_hex_byte(*high, *low).filter(u8::is_ascii) {
                    projected.push(decoded.to_ascii_lowercase());
                    remaining = tail;
                    continue;
                }
            }
            [b'\\', b'u' | b'U', first, second, third, fourth, tail @ ..] => {
                if let Some(decoded) =
                    decode_ascii_unicode_escape(&[*first, *second, *third, *fourth])
                {
                    projected.push(decoded.to_ascii_lowercase());
                    remaining = tail;
                    continue;
                }
            }
            _ => {}
        }
        let (byte, tail) = remaining
            .split_first()
            .expect("non-empty diagnostic projection input has a first byte");
        projected.push(if byte.is_ascii() {
            byte.to_ascii_lowercase()
        } else {
            b' '
        });
        remaining = tail;
    }
    projected
}

fn decode_ascii_unicode_escape(hex: &[u8]) -> Option<u8> {
    let mut value = 0_u16;
    for byte in hex {
        value = value.checked_mul(16)?;
        value = value.checked_add(u16::from(hex_nibble(*byte)?))?;
    }
    u8::try_from(value).ok().filter(u8::is_ascii)
}

fn decode_hex_byte(high: u8, low: u8) -> Option<u8> {
    hex_nibble(high)?
        .checked_mul(16)?
        .checked_add(hex_nibble(low)?)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some((byte - b'a') + 10),
        b'A'..=b'F' => Some((byte - b'A') + 10),
        _ => None,
    }
}

fn credential_key_before(value: &[u8], delimiter_index: usize) -> Option<(usize, String)> {
    let before_delimiter = &value[..delimiter_index];
    let end = before_delimiter
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\''))
        .map_or(0, |index| index + 1);
    let start = before_delimiter[..end]
        .iter()
        .rposition(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        .map_or(0, |index| index + 1);
    (start < end).then(|| {
        (
            start,
            String::from_utf8_lossy(&value[start..end]).into_owned(),
        )
    })
}

fn token_colon_has_credential_shape(value: &[u8], key_start: usize, value_start: usize) -> bool {
    let structured_key = key_start > 0
        && matches!(
            value[key_start - 1],
            b'"' | b'\'' | b'{' | b'[' | b'(' | b',' | b'&' | b'?'
        );
    let remainder = value[value_start..]
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .collect::<Vec<_>>();
    structured_key
        || remainder
            .first()
            .is_some_and(|byte| matches!(byte, b'"' | b'\''))
        || [b"bearer ".as_slice(), b"basic ", b"digest "]
            .into_iter()
            .any(|prefix| remainder.starts_with(prefix))
}

/// A value-free summary for command-line arguments at formatting boundaries.
///
/// The summary stores only an argument count and the presence of a deliberately
/// small set of exact, value-free flag names. It never retains raw arguments,
/// option values, paths, URLs, or command lines.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RedactedCommandArgs {
    count: usize,
    safe_flags_present: Vec<&'static str>,
}

impl RedactedCommandArgs {
    pub fn from_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut summary = Self::default();
        for arg in args {
            summary.count += 1;
            if let Some(name) = safe_standalone_flag_name(arg.as_ref()) {
                summary.record_safe_flag(name);
            }
        }
        summary
    }

    pub fn from_option_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut summary = Self::default();
        for name in names {
            summary.count += 1;
            if let Some(name) = safe_option_name(name.as_ref()) {
                summary.record_safe_flag(name);
            }
        }
        summary
    }

    pub const fn from_count(count: usize) -> Self {
        Self {
            count,
            safe_flags_present: Vec::new(),
        }
    }

    pub const fn count(&self) -> usize {
        self.count
    }

    fn record_safe_flag(&mut self, name: &'static str) {
        if !self.safe_flags_present.contains(&name) {
            self.safe_flags_present.push(name);
        }
    }
}

impl fmt::Debug for RedactedCommandArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactedCommandArgs")
            .field("count", &self.count)
            .field("safe_flags_present", &self.safe_flags_present)
            .finish()
    }
}

impl fmt::Display for RedactedCommandArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} command argument(s)", self.count)?;
        if !self.safe_flags_present.is_empty() {
            write!(
                formatter,
                " (safe flags present: {})",
                self.safe_flags_present.join(", ")
            )?;
        }
        Ok(())
    }
}

fn safe_standalone_flag_name(arg: &str) -> Option<&'static str> {
    match arg {
        "--fullscreen" | "--fs" => Some("fullscreen"),
        "--pause" => Some("pause"),
        "--ontop" => Some("ontop"),
        "--border" => Some("border"),
        "--force-window" => Some("force-window"),
        "--keep-open" => Some("keep-open"),
        _ => None,
    }
}

fn safe_option_name(name: &str) -> Option<&'static str> {
    match name {
        "fullscreen" | "fs" => Some("fullscreen"),
        "pause" => Some("pause"),
        "ontop" => Some("ontop"),
        "border" => Some("border"),
        "force-window" => Some("force-window"),
        "keep-open" => Some("keep-open"),
        _ => None,
    }
}

/// An owned secret whose ordinary formatting never exposes its contents.
///
/// This type intentionally does not implement `AsRef<str>`, `Deref`, or any
/// serialization traits. Callers must explicitly expose the value at an I/O
/// boundary.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    pub fn into_exposed_secret(self) -> String {
        self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_SECRET)
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED_SECRET)
    }
}

impl From<String> for SecretValue {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretValue {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        REDACTED_SECRET, RedactedCommandArgs, SecretValue, credential_key_before,
        decode_ascii_unicode_escape, decode_hex_byte, diagnostic_ascii_projection, hex_nibble,
        key_is_sensitive, sensitive_key_words, text_may_contain_credentials,
        token_colon_has_credential_shape,
    };

    #[test]
    fn structured_sensitive_key_aliases_share_one_canonical_policy() {
        for key in [
            "password",
            "access_token",
            "X-Plex-Token",
            "authTokenValue",
            "clientSecret",
            "room-password",
            "credentials",
            "futureCredential",
            "set-cookie",
            "x-api-key",
            "httpHeaders",
            "logicalMediaId",
            "request_id",
            "acceptedOperationId",
        ] {
            assert!(key_is_sensitive(key), "sensitive key {key:?}");
        }
        for key in [
            "",
            "monkey",
            "secretary",
            "tokenizer",
            "headerStyles",
            "request",
        ] {
            assert!(!key_is_sensitive(key), "ordinary key {key:?}");
        }
    }

    #[test]
    fn diagnostic_classifier_recognizes_escaped_and_prose_credential_fields() {
        for diagnostic in [
            r#"request failed: {"pass\u0077ord":"canary"}"#,
            r#"request failed: {"password"\u003a"canary"}"#,
            r#"request failed: password\u003dcanary"#,
            "request failed: access%5Ftoken=canary",
            "request failed with password: Bearer canary",
            "upstream response includes token=canary",
            "request failed (secret=canary)",
            "backend -> clientSecret: canary",
        ] {
            assert!(
                text_may_contain_credentials(diagnostic),
                "credential diagnostic {diagnostic:?}"
            );
        }
    }

    #[test]
    fn diagnostic_classifier_preserves_ordinary_parser_messages() {
        for diagnostic in [
            "unexpected token: EOF",
            "request failed: timeout",
            "parser: invalid syntax",
            "credential provider unavailable",
            "unexpected EOF while waiting for mpv IPC response (request_id=1)",
            "mpv request rejected (request_id=1): property not found",
            "hook operation failed (operation_id=7): client not found",
        ] {
            assert!(
                !text_may_contain_credentials(diagnostic),
                "ordinary diagnostic {diagnostic:?}"
            );
        }
    }

    #[test]
    fn sensitive_key_word_scanner_preserves_boundaries_without_empty_words() {
        let cases = [
            ("clientSecret", vec!["client", "secret"]),
            ("HTTP-password_value", vec!["http", "password", "value"]),
            ("token.value", vec!["token", "value"]),
            ("UPPER", vec!["upper"]),
            ("--", vec![]),
            ("", vec![]),
        ];

        for (key, expected) in cases {
            assert_eq!(
                sensitive_key_words(key).collect::<Vec<_>>(),
                expected,
                "key {key:?}"
            );
        }
    }

    #[test]
    fn diagnostic_ascii_projection_decodes_only_complete_ascii_escapes() {
        let cases = [
            ("AbC", b"abc".as_slice()),
            ("%41%5f%7A", b"a_z"),
            (r"\u0041\U005f\u007A", b"a_z"),
            ("%4", b"%4"),
            ("%GG", b"%gg"),
            (r"\u00", br"\u00"),
            (r"\u0080", br"\u0080"),
            (r"\x0041", br"\x0041"),
            ("AéZ", b"a  z"),
        ];

        for (diagnostic, expected) in cases {
            assert_eq!(
                diagnostic_ascii_projection(diagnostic),
                expected,
                "diagnostic {diagnostic:?}"
            );
        }
    }

    #[test]
    fn hex_decoders_cover_numeric_and_both_alpha_boundaries() {
        for (encoded, expected) in [
            (b'0', Some(0)),
            (b'9', Some(9)),
            (b'a', Some(10)),
            (b'f', Some(15)),
            (b'A', Some(10)),
            (b'F', Some(15)),
            (b'/', None),
            (b'g', None),
            (b'G', None),
        ] {
            assert_eq!(hex_nibble(encoded), expected, "nibble {encoded:?}");
        }

        assert_eq!(decode_hex_byte(b'0', b'0'), Some(0));
        assert_eq!(decode_hex_byte(b'4', b'1'), Some(b'A'));
        assert_eq!(decode_hex_byte(b'f', b'F'), Some(u8::MAX));
        assert_eq!(decode_hex_byte(b'G', b'1'), None);
        assert_eq!(decode_hex_byte(b'1', b'G'), None);

        assert_eq!(decode_ascii_unicode_escape(b"0000"), Some(0));
        assert_eq!(decode_ascii_unicode_escape(b"0041"), Some(b'A'));
        assert_eq!(decode_ascii_unicode_escape(b"007f"), Some(0x7f));
        assert_eq!(decode_ascii_unicode_escape(b"0080"), None);
        assert_eq!(decode_ascii_unicode_escape(b"0G41"), None);
    }

    #[test]
    fn credential_key_scanner_handles_empty_quoted_and_prefixed_boundaries() {
        let cases = [
            (b"=".as_slice(), 0, None),
            (b" password \t=".as_slice(), 11, Some((1, "password"))),
            (br#""token" :"#.as_slice(), 8, Some((1, "token"))),
            (b"prefix.password=".as_slice(), 15, Some((7, "password"))),
            (b"prefix.=".as_slice(), 7, None),
        ];

        for (value, delimiter_index, expected) in cases {
            assert_eq!(
                credential_key_before(value, delimiter_index),
                expected.map(|(start, key)| (start, key.to_owned())),
                "value {:?}",
                String::from_utf8_lossy(value)
            );
        }
    }

    #[test]
    fn token_colon_shape_distinguishes_each_independent_safe_branch() {
        assert!(token_colon_has_credential_shape(b"x(token: EOF", 2, 9));
        assert!(token_colon_has_credential_shape(b"token: 'value'", 0, 6));
        assert!(token_colon_has_credential_shape(
            b"token: bearer value",
            0,
            6
        ));
        assert!(token_colon_has_credential_shape(
            b"token: basic value",
            0,
            6
        ));
        assert!(token_colon_has_credential_shape(
            b"token: digest value",
            0,
            6
        ));
        assert!(!token_colon_has_credential_shape(b"token: EOF", 0, 6));
        assert!(!token_colon_has_credential_shape(b"xtoken: eof", 1, 7));
    }

    #[test]
    fn diagnostic_classifier_covers_token_colon_value_shapes_and_offsets() {
        for diagnostic in [
            r#"token: "secret""#,
            "token: 'secret'",
            "token: Bearer secret",
            "token: basic secret",
            "token: digest secret",
            r#"{"token":unquoted}"#,
            "prefix (token: EOF)",
        ] {
            assert!(
                text_may_contain_credentials(diagnostic),
                "credential diagnostic {diagnostic:?}"
            );
        }
        for diagnostic in ["token: EOF", "token: timeout", "token:", "tokenizer: EOF"] {
            assert!(
                !text_may_contain_credentials(diagnostic),
                "ordinary diagnostic {diagnostic:?}"
            );
        }
    }

    #[test]
    fn debug_and_display_are_always_redacted() {
        let secret = SecretValue::new("do-not-print-me");

        assert_eq!(format!("{secret:?}"), REDACTED_SECRET);
        assert_eq!(format!("{secret}"), REDACTED_SECRET);
    }

    #[test]
    fn secret_is_only_available_through_explicit_exposure() {
        let secret = SecretValue::new("boundary-value");

        assert_eq!(secret.expose_secret(), "boundary-value");
        assert_eq!(secret.into_exposed_secret(), "boundary-value");
    }

    #[test]
    fn blank_secret_checks_do_not_require_exposure() {
        assert!(SecretValue::new("").is_blank());
        assert!(SecretValue::new(" \t\r\n").is_blank());
        assert!(!SecretValue::new(" token ").is_blank());
    }

    #[test]
    fn command_argument_summary_never_retains_or_formats_values() {
        let canaries = [
            "--http-header-fields=Authorization: Bearer AUTHORIZATION_CANARY",
            "--cookies-file=C:/private/COOKIES_PATH_CANARY.txt",
            "https://media.example/video?Signature=SIGNED_URL_CANARY",
            "--fullscreen",
        ];

        let summary = RedactedCommandArgs::from_args(canaries);
        let debug = format!("{summary:?}");
        let display = summary.to_string();

        assert_eq!(summary.count(), 4);
        assert!(debug.contains("fullscreen"));
        assert!(display.contains("fullscreen"));
        for canary in canaries[..3].iter() {
            assert!(!debug.contains(canary));
            assert!(!display.contains(canary));
        }
        assert!(!debug.contains("AUTHORIZATION_CANARY"));
        assert!(!debug.contains("COOKIES_PATH_CANARY"));
        assert!(!debug.contains("SIGNED_URL_CANARY"));
    }

    #[test]
    fn command_argument_summary_recognizes_every_safe_standalone_flag() {
        let cases = [
            ("--fullscreen", "fullscreen"),
            ("--fs", "fullscreen"),
            ("--pause", "pause"),
            ("--ontop", "ontop"),
            ("--border", "border"),
            ("--force-window", "force-window"),
            ("--keep-open", "keep-open"),
        ];

        for (argument, expected_name) in cases {
            let summary = RedactedCommandArgs::from_args([argument]);

            assert_eq!(summary.count(), 1, "argument {argument}");
            assert_eq!(
                summary.safe_flags_present,
                vec![expected_name],
                "argument {argument}"
            );
            assert_eq!(
                summary.to_string(),
                format!("1 command argument(s) (safe flags present: {expected_name})"),
                "argument {argument}"
            );
        }
    }

    #[test]
    fn command_argument_summary_deduplicates_safe_aliases_in_first_seen_order() {
        let summary = RedactedCommandArgs::from_args([
            "--pause",
            "--fullscreen",
            "--fs",
            "--pause",
            "--keep-open",
        ]);

        assert_eq!(summary.count(), 5);
        assert_eq!(
            summary.safe_flags_present,
            vec!["pause", "fullscreen", "keep-open"]
        );
        assert_eq!(
            summary.to_string(),
            "5 command argument(s) (safe flags present: pause, fullscreen, keep-open)"
        );
    }

    #[test]
    fn option_name_summary_recognizes_only_exact_value_free_names() {
        let cases = [
            ("fullscreen", "fullscreen"),
            ("fs", "fullscreen"),
            ("pause", "pause"),
            ("ontop", "ontop"),
            ("border", "border"),
            ("force-window", "force-window"),
            ("keep-open", "keep-open"),
        ];

        for (option_name, expected_name) in cases {
            let summary = RedactedCommandArgs::from_option_names([option_name]);

            assert_eq!(summary.count(), 1, "option name {option_name}");
            assert_eq!(
                summary.safe_flags_present,
                vec![expected_name],
                "option name {option_name}"
            );
        }

        let rejected = [
            "",
            "Fullscreen",
            "--fullscreen",
            "fullscreen=yes",
            "keep_open",
            "cookies-file",
            "http-header-fields",
        ];
        for option_name in rejected {
            let summary = RedactedCommandArgs::from_option_names([option_name]);

            assert_eq!(summary.count(), 1, "option name {option_name:?}");
            assert!(
                summary.safe_flags_present.is_empty(),
                "option name {option_name:?}"
            );
            assert_eq!(summary.to_string(), "1 command argument(s)");
        }
    }

    #[test]
    fn option_name_summary_counts_all_names_and_deduplicates_aliases() {
        let summary = RedactedCommandArgs::from_option_names([
            "fullscreen",
            "fs",
            "unknown",
            "pause",
            "fullscreen",
        ]);

        assert_eq!(summary.count(), 5);
        assert_eq!(summary.safe_flags_present, vec!["fullscreen", "pause"]);
        assert_eq!(
            summary.to_string(),
            "5 command argument(s) (safe flags present: fullscreen, pause)"
        );
    }

    #[test]
    fn count_only_summary_preserves_the_exact_count_without_safe_flags() {
        let summary = RedactedCommandArgs::from_count(7);

        assert_eq!(summary.count(), 7);
        assert!(summary.safe_flags_present.is_empty());
        assert_eq!(summary.to_string(), "7 command argument(s)");
    }

    #[test]
    fn empty_secret_is_distinct_from_blank_nonempty_secret() {
        assert!(SecretValue::new("").is_empty());
        assert!(!SecretValue::new(" \t\r\n").is_empty());
        assert!(!SecretValue::new("token").is_empty());
    }

    #[test]
    fn string_and_str_conversions_preserve_the_exact_secret() {
        let owned = SecretValue::from(String::from("owned-secret"));
        let borrowed = SecretValue::from("borrowed-secret");

        assert_eq!(owned.expose_secret(), "owned-secret");
        assert_eq!(borrowed.expose_secret(), "borrowed-secret");
    }
}
