use std::process::Command;

pub(in crate::app) fn configure_gui_child_process(command: &mut Command) -> &mut Command {
    configure_gui_child_process_platform(command)
}

#[cfg(windows)]
fn configure_gui_child_process_platform(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn configure_gui_child_process_platform(command: &mut Command) -> &mut Command {
    command
}
