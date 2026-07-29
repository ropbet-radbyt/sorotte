use std::{collections::BTreeMap, fmt};

use serde_json::Value;

pub(crate) struct RedactedJsonValue<'a>(pub(crate) &'a Value);

impl fmt::Debug for RedactedJsonValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Object(object) => {
                let mut map = formatter.debug_map();
                for (key, value) in object {
                    if json_key_is_sensitive(key) {
                        map.entry(key, &RedactedSecret);
                    } else {
                        map.entry(key, &RedactedJsonValue(value));
                    }
                }
                map.finish()
            }
            Value::Array(values) => formatter
                .debug_list()
                .entries(values.iter().map(RedactedJsonValue))
                .finish(),
            Value::String(value) if text_may_contain_credentials(value) => {
                fmt::Debug::fmt(&RedactedSecret, formatter)
            }
            value => fmt::Debug::fmt(value, formatter),
        }
    }
}

pub(crate) struct RedactedOptionalJsonValue<'a>(pub(crate) Option<&'a Value>);

impl fmt::Debug for RedactedOptionalJsonValue<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => formatter
                .debug_tuple("Some")
                .field(&RedactedJsonValue(value))
                .finish(),
            None => formatter.write_str("None"),
        }
    }
}

pub(crate) struct RedactedOptionalText<'a>(pub(crate) Option<&'a str>);

impl fmt::Debug for RedactedOptionalText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(_) => formatter
                .debug_tuple("Some")
                .field(&RedactedSecret)
                .finish(),
            None => formatter.write_str("None"),
        }
    }
}

pub(crate) struct RedactedOptionalSensitiveText<'a>(pub(crate) Option<&'a str>);

impl fmt::Debug for RedactedOptionalSensitiveText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => formatter
                .debug_tuple("Some")
                .field(&RedactedSensitiveText(value))
                .finish(),
            None => formatter.write_str("None"),
        }
    }
}

pub(crate) struct RedactedSensitiveText<'a>(pub(crate) &'a str);

impl fmt::Debug for RedactedSensitiveText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if text_may_contain_credentials(self.0) {
            fmt::Debug::fmt(&RedactedSecret, formatter)
        } else {
            fmt::Debug::fmt(self.0, formatter)
        }
    }
}

pub(crate) struct RedactedTextList<'a>(pub(crate) &'a [String]);

impl fmt::Debug for RedactedTextList<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_list()
            .entries(self.0.iter().map(String::as_str).map(RedactedSensitiveText))
            .finish()
    }
}

pub(crate) struct RedactedJsonMap<'a>(pub(crate) &'a BTreeMap<String, Value>);

impl fmt::Debug for RedactedJsonMap<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = formatter.debug_map();
        for (key, value) in self.0 {
            if json_key_is_sensitive(key) {
                map.entry(key, &RedactedSecret);
            } else {
                map.entry(key, &RedactedJsonValue(value));
            }
        }
        map.finish()
    }
}

struct RedactedSecret;

impl fmt::Debug for RedactedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(sorotte_secret::REDACTED_SECRET)
    }
}

fn json_key_is_sensitive(key: &str) -> bool {
    sorotte_secret::key_is_sensitive(key)
}

pub(crate) fn text_may_contain_credentials(value: &str) -> bool {
    sorotte_secret::text_may_contain_credentials(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_value_debug_preserves_ordinary_values_and_redacts_credentials() {
        let ordinary = Value::String("playback timeout".to_owned());
        let credential = Value::String("request failed: password=canary".to_owned());

        assert_eq!(
            format!("{:?}", RedactedJsonValue(&ordinary)),
            "String(\"playback timeout\")"
        );
        assert_eq!(
            format!("{:?}", RedactedJsonValue(&credential)),
            sorotte_secret::REDACTED_SECRET
        );
    }

    #[test]
    fn json_value_debug_applies_key_and_value_redaction_recursively() {
        let value = serde_json::json!({
            "displayName": "alice",
            "nested": [{
                "status": "ready",
                "password": "password-canary"
            }],
            "diagnostic": "access_token=token-canary"
        });

        let debug = format!("{:?}", RedactedJsonValue(&value));
        assert!(debug.contains("\"displayName\": String(\"alice\")"));
        assert!(debug.contains("\"status\": String(\"ready\")"));
        assert_eq!(debug.matches(sorotte_secret::REDACTED_SECRET).count(), 2);
        assert!(!debug.contains("password-canary"));
        assert!(!debug.contains("token-canary"));
    }

    #[test]
    fn optional_json_debug_distinguishes_some_and_none_exactly() {
        let ordinary = Value::String("ready".to_owned());
        let credential = Value::String("token=canary".to_owned());

        assert_eq!(
            format!("{:?}", RedactedOptionalJsonValue(Some(&ordinary))),
            "Some(String(\"ready\"))"
        );
        assert_eq!(
            format!("{:?}", RedactedOptionalJsonValue(Some(&credential))),
            "Some(<redacted>)"
        );
        assert_eq!(format!("{:?}", RedactedOptionalJsonValue(None)), "None");
    }

    #[test]
    fn optional_text_debug_is_always_value_free() {
        assert_eq!(
            format!("{:?}", RedactedOptionalText(Some("ordinary"))),
            "Some(<redacted>)"
        );
        assert_eq!(format!("{:?}", RedactedOptionalText(None)), "None");
    }

    #[test]
    fn optional_sensitive_text_debug_classifies_some_and_preserves_none() {
        assert_eq!(
            format!(
                "{:?}",
                RedactedOptionalSensitiveText(Some("playback timeout"))
            ),
            "Some(\"playback timeout\")"
        );
        assert_eq!(
            format!(
                "{:?}",
                RedactedOptionalSensitiveText(Some("access_token=canary"))
            ),
            "Some(<redacted>)"
        );
        assert_eq!(format!("{:?}", RedactedOptionalSensitiveText(None)), "None");
    }

    #[test]
    fn text_list_debug_classifies_each_value_independently() {
        let values = vec![
            "ready".to_owned(),
            "request failed: password=canary".to_owned(),
            "buffering".to_owned(),
        ];

        assert_eq!(
            format!("{:?}", RedactedTextList(&values)),
            "[\"ready\", <redacted>, \"buffering\"]"
        );
    }

    #[test]
    fn json_map_debug_preserves_ordinary_keys_and_redacts_sensitive_keys() {
        let values = BTreeMap::from([
            ("displayName".to_owned(), Value::String("alice".to_owned())),
            (
                "vendorAccessToken".to_owned(),
                Value::String("token-canary".to_owned()),
            ),
        ]);

        assert_eq!(
            format!("{:?}", RedactedJsonMap(&values)),
            "{\"displayName\": String(\"alice\"), \"vendorAccessToken\": <redacted>}"
        );
    }

    #[test]
    fn classifier_forwarders_preserve_both_boolean_outcomes() {
        assert!(!json_key_is_sensitive("displayName"));
        assert!(json_key_is_sensitive("vendorAccessToken"));
        assert!(!text_may_contain_credentials("playback timeout"));
        assert!(text_may_contain_credentials("access_token=canary"));
    }
}
