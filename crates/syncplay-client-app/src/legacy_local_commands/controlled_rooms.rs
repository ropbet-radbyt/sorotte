use std::sync::atomic::{AtomicU64, Ordering};

const CONTROL_ROOM_HASH_LEN: usize = 12;
static ROOM_PASSWORD_NONCE: AtomicU64 = AtomicU64::new(0);

pub fn controlled_room_base_name_legacy_compatible(room: &str) -> String {
    if !room.starts_with('+') {
        return room.to_owned();
    }

    let Some(room_without_prefix) = room.strip_prefix('+') else {
        return room.to_owned();
    };
    let Some((room_base, hash_suffix)) = room_without_prefix.rsplit_once(':') else {
        return room.to_owned();
    };
    if room_base.is_empty()
        || hash_suffix.len() != CONTROL_ROOM_HASH_LEN
        || !hash_suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return room.to_owned();
    }
    room_base.to_owned()
}

pub fn generate_room_password_legacy_compatible() -> String {
    fn next_seed() -> u64 {
        let nanos_since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let nonce = ROOM_PASSWORD_NONCE.fetch_add(1, Ordering::Relaxed);
        nanos_since_epoch
            ^ nonce.rotate_left(17)
            ^ ((std::process::id() as u64) << 32)
            ^ 0x9E37_79B9_7F4A_7C15
    }

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *seed
    }

    fn next_letter(seed: &mut u64) -> char {
        let value = (lcg(seed) % 26) as u8;
        (b'A' + value) as char
    }

    fn next_digit(seed: &mut u64) -> char {
        let value = (lcg(seed) % 10) as u8;
        (b'0' + value) as char
    }

    let mut seed = next_seed();
    format!(
        "{}{}-{}{}{}-{}{}{}",
        next_letter(&mut seed),
        next_letter(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed),
        next_digit(&mut seed)
    )
}
