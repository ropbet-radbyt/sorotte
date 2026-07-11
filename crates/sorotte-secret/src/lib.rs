use std::fmt;

pub const REDACTED_SECRET: &str = "<redacted>";

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
    use super::{REDACTED_SECRET, RedactedCommandArgs, SecretValue};

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
}
