mod helpers;
mod merge;
mod parser;
mod paths;
mod transaction;
#[cfg(windows)]
mod windows_security;
mod writer;

pub use parser::parse_sorotte_ini_stored_client_settings_mvp;
pub use paths::{
    clear_sorotte_ini_stored_client_settings_mvp_at_path, create_private_directory,
    edit_sorotte_ini_stored_client_settings_mvp_at_path,
    load_sorotte_ini_stored_client_settings_mvp_from_path,
    merge_sorotte_ini_stored_client_settings_mvp_at_path,
    relocate_sorotte_ini_stored_client_settings_mvp_at_path,
    update_sorotte_ini_stored_client_settings_mvp_at_path,
    upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity_at_path,
    write_sorotte_ini_contents_atomically_at_path,
};
pub(crate) use paths::{
    ensure_sorotte_ini_contents_at_path, read_sorotte_ini_contents_consistently_at_path,
    update_sorotte_ini_contents_at_path,
};
pub use writer::{
    upsert_sorotte_ini_stored_client_settings_mvp,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
};

#[cfg(test)]
pub(crate) fn on_next_settings_lock_contention(hook: impl FnOnce() + 'static) {
    transaction::CONTENTION_HOOK.with(|next| *next.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
mod duplicate_tests;
#[cfg(test)]
mod read_transaction_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transaction_tests;
#[cfg(all(test, windows))]
mod windows_tests;
