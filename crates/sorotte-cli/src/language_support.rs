use std::sync::{Mutex, OnceLock};

use sorotte_client_app::app_boundary::{
    language::resolve_runtime_language_tag, state::StoredClientSettings,
};

use crate::client_args::SyncplayClientArgOverrides;

static RUNTIME_LANGUAGE_TAG: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn runtime_language_tag_storage() -> &'static Mutex<Option<String>> {
    RUNTIME_LANGUAGE_TAG.get_or_init(|| Mutex::new(None))
}

pub(super) fn set_runtime_language_for_process(language: Option<String>) {
    let mut guard = runtime_language_tag_storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = language;
}

pub(super) fn current_runtime_language_tag() -> Option<String> {
    runtime_language_tag_storage()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub(super) fn resolved_runtime_language_tag(
    overrides: &SyncplayClientArgOverrides,
    stored_settings: Option<&StoredClientSettings>,
) -> Option<String> {
    resolve_runtime_language_tag(
        overrides.language.as_deref(),
        stored_settings.and_then(|settings| settings.language.as_deref()),
    )
    .map(ToOwned::to_owned)
}
