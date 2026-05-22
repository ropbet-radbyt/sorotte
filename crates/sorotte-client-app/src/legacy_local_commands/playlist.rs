use sorotte_client_core::ClientSession;

pub fn playlist_index_in_bounds_legacy_compatible(session: &ClientSession, index: i64) -> bool {
    if index < 0 {
        return false;
    }
    let Ok(index) = usize::try_from(index) else {
        return false;
    };
    session
        .current_room_playlist()
        .is_some_and(|playlist| index < playlist.files.len())
}
