use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sorotte_client_app::app_boundary::{
    persistence::update_sorotte_ini_stored_client_settings_mvp_at_path,
    state::StoredClientSettingsMvp,
};

use crate::client_args::LegacyClientArgOverrides;
use crate::config_paths::resolve_sorotte_cli_config_path;

pub(super) const LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS: u64 = 7 * 86400;

pub(super) fn legacy_utc_timestamp_string_legacy_compatible(now: SystemTime) -> String {
    let duration = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_seconds = duration.as_secs() as i64;
    let days_since_epoch = total_seconds.div_euclid(86_400);
    let seconds_of_day = total_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days_since_unix_epoch_legacy_compatible(days_since_epoch);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let millis = duration.subsec_millis();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
}

pub(super) fn parse_legacy_utc_timestamp_legacy_compatible(value: &str) -> Option<SystemTime> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 23
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
    {
        return None;
    }

    let year = value[0..4].parse::<i64>().ok()?;
    let month = value[5..7].parse::<u32>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let hour = value[11..13].parse::<u64>().ok()?;
    let minute = value[14..16].parse::<u64>().ok()?;
    let second = value[17..19].parse::<u64>().ok()?;
    let millis = value[20..23].parse::<u64>().ok()?;

    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
        || millis > 999
    {
        return None;
    }

    let days_since_epoch =
        days_since_unix_epoch_from_civil_legacy_compatible(year, month as i64, day as i64);
    if days_since_epoch < 0 {
        return None;
    }

    let total_seconds = days_since_epoch as u64 * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(UNIX_EPOCH + Duration::from_secs(total_seconds) + Duration::from_millis(millis))
}

fn civil_from_days_since_unix_epoch_legacy_compatible(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

fn days_since_unix_epoch_from_civil_legacy_compatible(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let yoe = adjusted_year - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(super) fn should_run_headless_automatic_update_check_legacy_compatible(
    settings: Option<&StoredClientSettingsMvp>,
    now: SystemTime,
) -> bool {
    let Some(settings) = settings else {
        return false;
    };
    if settings.check_for_updates_automatically != Some(true) {
        return false;
    }
    let Some(last_checked) = settings
        .last_checked_for_updates
        .as_deref()
        .and_then(parse_legacy_utc_timestamp_legacy_compatible)
    else {
        return true;
    };

    now.duration_since(last_checked)
        .map(|elapsed| elapsed.as_secs() > LEGACY_AUTOMATIC_UPDATE_CHECK_FREQUENCY_SECONDS)
        .unwrap_or(false)
}

pub(super) fn persist_sorotte_cli_last_checked_for_updates_setting_legacy_compatible(
    timestamp: &str,
) -> anyhow::Result<()> {
    let Some(path) = resolve_sorotte_cli_config_path() else {
        return Ok(());
    };
    update_sorotte_ini_stored_client_settings_mvp_at_path(&path, |settings| {
        settings.last_checked_for_updates = Some(timestamp.to_owned());
    })
}

pub(super) fn apply_headless_automatic_update_check_legacy_compatible(
    overrides: &LegacyClientArgOverrides,
    settings: Option<&StoredClientSettingsMvp>,
) {
    let now = SystemTime::now();
    if !should_run_headless_automatic_update_check_legacy_compatible(settings, now) {
        return;
    }

    eprintln!(
        "info: legacy automatic update check is due; sorotte-cli records the headless check timestamp but does not perform GUI update dialogs or remote update probing"
    );
    if overrides.no_store {
        return;
    }

    let timestamp = legacy_utc_timestamp_string_legacy_compatible(now);
    if let Err(error) =
        persist_sorotte_cli_last_checked_for_updates_setting_legacy_compatible(&timestamp)
    {
        eprintln!("warning: failed to persist legacy lastCheckedForUpdates setting: {error}");
    }
}
