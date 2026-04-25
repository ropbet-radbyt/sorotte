use super::startup::{
    gui_startup_actions_from_lookup, gui_startup_host_and_settings, load_gui_ui_state_from_lookup,
    run_gui_host_with_startup_actions_and_gui_state,
};
use super::startup_support::env_trimmed;

pub(super) fn run_syncplay_gui() {
    let (mut host, settings) = match gui_startup_host_and_settings() {
        Ok(startup) => startup,
        Err(error) => {
            eprintln!("syncplay-gui failed to configure startup runtime: {error}");
            std::process::exit(1);
        }
    };
    let persisted_ui_state = match load_gui_ui_state_from_lookup(&env_trimmed) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("syncplay-gui failed to load legacy GUI state: {error}");
            std::process::exit(1);
        }
    };
    let mut merged_settings = settings.clone();
    if let Some(persisted_ui_state) = persisted_ui_state.as_ref() {
        persisted_ui_state.merge_into_startup_settings(&mut merged_settings);
    }
    let startup_actions = gui_startup_actions_from_lookup(env_trimmed, &merged_settings);
    if let Err(error) = run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        persisted_ui_state.as_ref(),
        startup_actions,
        &mut host,
    ) {
        eprintln!("syncplay-gui failed to start: {error}");
        std::process::exit(1);
    }
}
