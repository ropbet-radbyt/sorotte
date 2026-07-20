use super::*;

pub(super) fn normalize_menu_label(raw_label: &str) -> String {
    raw_label.replace('&', "").trim().to_owned()
}

pub(super) fn verify_menu_contract(menu_labels: &[String]) -> Result<(), String> {
    let normalized = menu_labels
        .iter()
        .map(|label| normalize_menu_label(label))
        .collect::<Vec<_>>();
    let required = ["File", "Playback", "Advanced", "Window", "Help"];
    for expected in required {
        if !normalized.iter().any(|label| label == expected) {
            return Err(format!(
                "main window menu is missing required top-level entry {expected:?}; observed: {}",
                normalized.join(", ")
            ));
        }
    }
    Ok(())
}

pub(super) fn verify_accessibility_contract(accessible_names: &[String]) -> Result<(), String> {
    if accessible_names.is_empty() {
        return Err("accessibility tree did not expose any named elements".to_owned());
    }

    let required_labels = ["File", "Playback", "Advanced", "Window", "Help"];
    for required_label in required_labels {
        if !accessible_names.iter().any(|name| name == required_label) {
            return Err(format!(
                "accessibility tree is missing required top-level label {required_label:?}"
            ));
        }
    }

    if !accessible_names
        .iter()
        .any(|name| name == "view: setup" || name == "view: room")
    {
        return Err(
            "accessibility tree is missing a known view indicator (expected 'view: setup' or 'view: room')"
                .to_owned(),
        );
    }

    Ok(())
}

pub(super) fn contains_accessible_name(accessible_names: &[String], expected: &str) -> bool {
    accessible_names.iter().any(|name| name == expected)
}

pub(super) fn contains_accessible_name_fragment(
    accessible_names: &[String],
    expected_fragment: &str,
) -> bool {
    accessible_names
        .iter()
        .any(|name| name.contains(expected_fragment))
}

pub(super) fn render_accessible_name_snapshot_for_patterns(
    accessible_names: &[String],
    patterns: &[&str],
) -> String {
    let snapshot = accessible_names
        .iter()
        .filter(|name| patterns.iter().any(|pattern| name.contains(pattern)))
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>();
    if snapshot.is_empty() {
        "none".to_owned()
    } else {
        snapshot.join(", ")
    }
}

pub(super) fn wait_for_accessible_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if contains_accessible_name(&names, expected_name) {
                    return Ok(());
                }
                last_snapshot = Some(render_accessible_name_snapshot_for_patterns(
                    &names,
                    &[
                        "view:",
                        "self=",
                        "ready=",
                        "controller=",
                        "Status",
                        "Busy",
                        "Save",
                        "Reload",
                        "Connection / Port",
                        "Timeout",
                        "Warning",
                        "Interval",
                        "Media Search",
                        "view: setup",
                    ],
                ));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for accessibility name {expected_name:?}; last accessibility read error: {error}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            } else {
                Err(format!(
                    "timed out waiting for accessibility name {expected_name:?}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn wait_for_any_accessible_name<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_names: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if let Some(found) = expected_names
                    .iter()
                    .find(|expected| contains_accessible_name(&names, expected))
                {
                    return Ok((*found).to_owned());
                }
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            let expected_list = expected_names
                .iter()
                .map(|name| format!("{name:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for one of [{expected_list}] in accessibility tree; last accessibility read error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for one of [{expected_list}] in accessibility tree"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn wait_for_accessible_name_fragment<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    expected_fragment: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    let mut last_snapshot = None;
    loop {
        match driver.accessible_names(window) {
            Ok(names) => {
                if contains_accessible_name_fragment(&names, expected_fragment) {
                    return Ok(());
                }
                last_snapshot = Some(render_accessible_name_snapshot_for_patterns(
                    &names,
                    &[
                        expected_fragment,
                        "view:",
                        "self=",
                        "ready=",
                        "controller=",
                        "Status",
                        "Busy",
                        "Media Search",
                    ],
                ));
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for accessibility name containing {expected_fragment:?}; last accessibility read error: {error}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            } else {
                Err(format!(
                    "timed out waiting for accessibility name containing {expected_fragment:?}; last snapshot: {}",
                    last_snapshot.unwrap_or_else(|| "unavailable".to_owned())
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn invoke_named_control_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_snapshot = None;
    loop {
        match driver.invoke_named_control(window, name, control_kind) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    let snapshot = driver
                        .accessible_names(window)
                        .map(|names| {
                            render_accessible_name_snapshot_for_patterns(
                                &names,
                                &[
                                    name,
                                    "Save",
                                    "Discard changes",
                                    "Reload",
                                    "Configuration",
                                    "view:",
                                ],
                            )
                        })
                        .unwrap_or_else(|_| "unavailable".to_owned());
                    return Err(format!(
                        "timed out invoking {} named {name:?}; last error: {error}; last snapshot: {}",
                        control_kind.label(),
                        if last_snapshot.is_some() {
                            last_snapshot.take().unwrap()
                        } else {
                            snapshot
                        }
                    ));
                }
                last_snapshot = driver.accessible_names(window).ok().map(|names| {
                    render_accessible_name_snapshot_for_patterns(
                        &names,
                        &[
                            name,
                            "Save",
                            "Discard changes",
                            "Reload",
                            "Configuration",
                            "view:",
                        ],
                    )
                });
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn invoke_menu_command_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    menu_name: &str,
    command_name: &str,
    command_kind: NativeControlKind,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let _ = driver.invoke_named_control(window, menu_name, NativeControlKind::Any);
        thread::sleep(Duration::from_millis(100));
        match driver.invoke_named_control(window, command_name, command_kind) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out invoking menu command {menu_name:?}->{command_name:?}; last error: {error}"
                    ));
                }
            }
        }
        thread::sleep(Duration::from_millis(80));
    }
}

pub(super) fn wait_for_named_control_count<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    name: &str,
    control_kind: NativeControlKind,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    loop {
        match driver.count_named_controls(window, name, control_kind) {
            Ok(count) if count == expected_count => return Ok(()),
            Ok(_) => {}
            Err(error) => last_error = Some(error),
        }
        if Instant::now() >= deadline {
            return if let Some(error) = last_error {
                Err(format!(
                    "timed out waiting for {expected_count} controls named {name:?}; last count error: {error}"
                ))
            } else {
                Err(format!(
                    "timed out waiting for {expected_count} controls named {name:?}"
                ))
            };
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn invoke_menu_command_with_fallback<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    menu_name: &str,
    command_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    if let Err(primary_error) = invoke_menu_command_with_wait(
        driver,
        window,
        menu_name,
        command_name,
        NativeControlKind::MenuItem,
        timeout,
    ) {
        invoke_menu_command_with_wait(
            driver,
            window,
            menu_name,
            command_name,
            NativeControlKind::Any,
            timeout,
        )
        .map_err(|fallback_error| {
            format!(
                "failed to invoke {menu_name} -> {command_name} through menu item ({primary_error}); fallback also failed: {fallback_error}"
            )
        })
    } else {
        Ok(())
    }
}

pub(super) fn navigate_to_view_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    button_name: &str,
    view_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    if let Ok(accessible_names) = driver.accessible_names(window)
        && contains_accessible_name(&accessible_names, view_name)
    {
        return Ok(());
    }

    let view_timeout = timeout.min(Duration::from_millis(800));
    invoke_named_control_with_wait(
        driver,
        window,
        button_name,
        NativeControlKind::Button,
        timeout,
    )
    .and_then(|_| wait_for_accessible_name(driver, window, view_name, view_timeout))
}

pub(super) fn navigate_to_view_with_fallback<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    button_name: &str,
    view_name: &str,
    fallback_menu_name: &str,
    fallback_command_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let sidebar_result =
        navigate_to_view_with_wait(driver, window, button_name, view_name, timeout);
    if sidebar_result.is_ok() {
        return Ok(());
    }

    let sidebar_error = sidebar_result.err().unwrap_or_else(|| {
        format!("sidebar navigation to {view_name:?} did not complete successfully")
    });
    invoke_menu_command_with_fallback(
        driver,
        window,
        fallback_menu_name,
        fallback_command_name,
        timeout,
    )
    .map_err(|menu_error| {
        format!(
            "failed to navigate to {view_name:?}; sidebar attempt failed: {sidebar_error}; menu fallback failed: {menu_error}"
        )
    })?;
    wait_for_accessible_name(driver, window, view_name, timeout).map_err(|wait_error| {
        format!(
            "menu fallback reached {fallback_menu_name} -> {fallback_command_name}, but {view_name:?} never appeared after sidebar failure ({sidebar_error}): {wait_error}"
        )
    })
}

pub(super) fn select_top_tab_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    tab_name: &str,
    expected_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    if wait_for_accessible_name(
        driver,
        window,
        expected_name,
        timeout.min(Duration::from_millis(500)),
    )
    .is_ok()
    {
        return Ok(());
    }

    invoke_named_control_with_wait(driver, window, tab_name, NativeControlKind::Button, timeout)
        .map_err(|error| {
            format!("failed to activate top tab {tab_name:?} before waiting for {expected_name:?}: {error}")
        })?;
    wait_for_accessible_name(driver, window, expected_name, timeout).map(|_| ())
}
