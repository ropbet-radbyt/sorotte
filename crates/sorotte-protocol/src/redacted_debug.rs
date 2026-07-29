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
