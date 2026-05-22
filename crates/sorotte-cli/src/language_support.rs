use std::sync::{Mutex, OnceLock};

use sorotte_client_app::app_boundary::{
    language::resolve_legacy_runtime_language_tag_legacy_compatible, state::StoredClientSettingsMvp,
};

use crate::client_args::LegacyClientArgOverrides;

static LEGACY_RUNTIME_LANGUAGE_TAG_UTC_SAFE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn legacy_runtime_language_tag_storage_legacy_compatible() -> &'static Mutex<Option<String>> {
    LEGACY_RUNTIME_LANGUAGE_TAG_UTC_SAFE.get_or_init(|| Mutex::new(None))
}

pub(super) fn set_legacy_runtime_language_for_process_legacy_compatible(language: Option<String>) {
    let mut guard = legacy_runtime_language_tag_storage_legacy_compatible()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = language;
}

pub(super) fn current_legacy_runtime_language_tag_legacy_compatible() -> Option<String> {
    legacy_runtime_language_tag_storage_legacy_compatible()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub(super) fn resolved_legacy_runtime_language_tag_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
    stored_settings: Option<&StoredClientSettingsMvp>,
) -> Option<String> {
    resolve_legacy_runtime_language_tag_legacy_compatible(
        overrides.language.as_deref(),
        stored_settings.and_then(|settings| settings.language.as_deref()),
    )
    .map(ToOwned::to_owned)
}
