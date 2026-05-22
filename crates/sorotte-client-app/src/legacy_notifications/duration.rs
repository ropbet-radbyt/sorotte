const ROUND_HALF_EPSILON: f64 = 1e-12;

fn round_half_to_even(value: f64) -> f64 {
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

pub fn format_duration_legacy(time_seconds: f64) -> String {
    let sign = if time_seconds < 0.0 { "-" } else { "" };
    let rounded_seconds = round_half_to_even(time_seconds.abs()) as u64;

    let mut weeks = rounded_seconds / 604_800;
    let title = if weeks > 0 {
        let title = weeks;
        weeks = 0;
        title
    } else {
        0
    };
    let days = (rounded_seconds % 604_800) / 86_400;
    let hours = (rounded_seconds % 86_400) / 3_600;
    let minutes = (rounded_seconds % 3_600) / 60;
    let seconds = rounded_seconds % 60;

    let mut formatted = if weeks > 0 {
        format!("{sign}{weeks}w, {days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if days > 0 {
        format!("{sign}{days}d, {hours:02}:{minutes:02}:{seconds:02}")
    } else if hours > 0 {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{minutes:02}:{seconds:02}")
    };

    if title > 0 {
        formatted.push_str(&format!(" (Title {title})"));
    }

    formatted
}
