use std::fmt;

pub const REDACTED_SECRET: &str = "<redacted>";

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
    use super::{REDACTED_SECRET, SecretValue};

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
}
