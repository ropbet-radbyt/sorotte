use super::startup::{
    gui_startup_actions_from_env, gui_startup_host_and_settings, load_gui_ui_state_from_env,
    run_gui_host_with_startup_actions_and_gui_state,
};

pub(super) fn run_sorotte_gui() {
    let (mut host, settings) = match gui_startup_host_and_settings() {
        Ok(startup) => startup,
        Err(error) => {
            exit_after_startup_error(format!(
                "sorotte-gui failed to configure startup runtime: {error}"
            ));
        }
    };
    let persisted_ui_state = match load_gui_ui_state_from_env() {
        Ok(state) => state,
        Err(error) => {
            exit_after_startup_error(format!(
                "sorotte-gui failed to load legacy GUI state: {error}"
            ));
        }
    };
    let mut merged_settings = settings.clone();
    if let Some(persisted_ui_state) = persisted_ui_state.as_ref() {
        persisted_ui_state.merge_into_startup_settings(&mut merged_settings);
    }
    let startup_actions = gui_startup_actions_from_env(&merged_settings);
    if let Err(error) = run_gui_host_with_startup_actions_and_gui_state(
        &settings,
        persisted_ui_state.as_ref(),
        startup_actions,
        &mut host,
    ) {
        exit_after_startup_error(format!("sorotte-gui failed to start: {error}"));
    }
}

fn exit_after_startup_error(message: String) -> ! {
    eprintln!("{message}");
    show_startup_error(&message);
    std::process::exit(1);
}

#[cfg(target_os = "windows")]
fn show_startup_error(message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_title("Sorotte GUI")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(not(target_os = "windows"))]
fn show_startup_error(_message: &str) {}
