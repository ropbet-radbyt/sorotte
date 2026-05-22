use super::*;

pub(crate) fn apply_stored_legacy_startup_player_defaults_if_arg_absent(
    overrides: &mut LegacyClientArgOverrides,
    settings: &StoredClientSettingsMvp,
) {
    if overrides.player_path.is_none()
        && let Some(player_path) = settings.player_path.as_deref()
        && !player_path.is_empty()
    {
        overrides.player_path = Some(player_path.to_owned());
    }
    if let Some(player_path) = overrides.player_path.as_deref()
        && let Some(per_player_arguments) = settings.per_player_arguments.as_ref()
        && let Some(args) = lookup_stored_per_player_arguments_for_player_path_legacy_compatible(
            per_player_arguments,
            player_path,
        )
    {
        if overrides.player_args.is_empty() {
            overrides.player_args = args.clone();
        } else {
            overrides.player_args.extend(args.iter().cloned());
        }
    }
}

fn lookup_stored_per_player_arguments_for_player_path_legacy_compatible<'a>(
    per_player_arguments: &'a BTreeMap<String, Vec<String>>,
    player_path: &str,
) -> Option<&'a Vec<String>> {
    if let Some(args) = per_player_arguments.get(player_path) {
        return Some(args);
    }
    let normalized_player_path =
        normalize_player_path_for_stored_per_player_arguments_lookup_legacy_compatible(
            player_path,
        )?;
    per_player_arguments
        .iter()
        .find_map(|(stored_player_path, args)| {
            let normalized_stored_player_path =
                normalize_player_path_for_stored_per_player_arguments_lookup_legacy_compatible(
                    stored_player_path,
                )?;
            (normalized_stored_player_path == normalized_player_path).then_some(args)
        })
}

pub(crate) fn normalize_player_path_for_stored_per_player_arguments_lookup_legacy_compatible(
    player_path: &str,
) -> Option<String> {
    let trimmed = player_path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let slash_normalized = trimmed.replace('\\', "/");
    if looks_like_windows_player_path_legacy_compatible(trimmed) {
        return Some(slash_normalized.to_ascii_lowercase());
    }
    Some(slash_normalized)
}

fn looks_like_windows_player_path_legacy_compatible(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return true;
    }
    path.starts_with("\\\\") || path.starts_with("//") || path.contains('\\')
}
