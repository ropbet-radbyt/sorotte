mod helpers;
mod parser;
mod paths;
mod writer;

pub use parser::parse_sorotte_ini_stored_client_settings_mvp;
pub use paths::{
    clear_sorotte_ini_stored_client_settings_mvp_at_path,
    load_sorotte_ini_stored_client_settings_mvp_from_path,
    update_sorotte_ini_stored_client_settings_mvp_at_path,
    upsert_sorotte_ini_stored_client_settings_mvp_at_path,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity_at_path,
};
pub use writer::{
    upsert_sorotte_ini_stored_client_settings_mvp,
    upsert_sorotte_ini_stored_client_settings_mvp_clearing_plex_identity,
};

#[cfg(test)]
mod tests;
