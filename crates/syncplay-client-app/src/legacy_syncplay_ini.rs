mod helpers;
mod parser;
mod paths;
mod writer;

pub use parser::parse_syncplay_ini_stored_client_settings_mvp;
pub use paths::{
    clear_syncplay_ini_stored_client_settings_mvp_at_path,
    load_syncplay_ini_stored_client_settings_mvp_from_path,
    update_syncplay_ini_stored_client_settings_mvp_at_path,
    upsert_syncplay_ini_stored_client_settings_mvp_at_path,
};
pub use writer::upsert_syncplay_ini_stored_client_settings_mvp;

#[cfg(test)]
mod tests;
