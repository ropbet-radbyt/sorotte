use super::*;

pub(super) const MENU_SOURCE_UIA_ACCESSKIT: &str = "uia-accesskit";
pub(super) const FILE_MENU_AUTOMATION_ID: &str = "menu.section.file";
pub(super) const ADVANCED_MENU_AUTOMATION_ID: &str = "menu.section.advanced";
pub(super) const HELP_MENU_AUTOMATION_ID: &str = "menu.section.help";
pub(super) const OPEN_MEDIA_MENU_AUTOMATION_ID: &str = "menu.open_media";
pub(super) const EXIT_MENU_AUTOMATION_ID: &str = "menu.exit";
pub(super) const ABOUT_MENU_AUTOMATION_ID: &str = "menu.about";
pub(super) const TLS_CERTIFICATES_MENU_AUTOMATION_ID: &str = "menu.tls_certificates";

const REQUIRED_MENU_SECTIONS: [(&str, &str); 5] = [
    (FILE_MENU_AUTOMATION_ID, "File"),
    ("menu.section.playback", "Playback"),
    ("menu.section.advanced", "Advanced"),
    ("menu.section.window", "Window"),
    (HELP_MENU_AUTOMATION_ID, "Help"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeMenuEvidence {
    pub(super) labels: Vec<String>,
    pub(super) automation_ids: Vec<String>,
}

pub(super) fn verify_menu_contract(
    accessibility_nodes: &[NativeAccessibilityNode],
) -> Result<NativeMenuEvidence, String> {
    let menu_nodes = accessibility_nodes
        .iter()
        .filter(|node| node.automation_id.starts_with("menu.section."))
        .collect::<Vec<_>>();
    let expected_ids = REQUIRED_MENU_SECTIONS
        .iter()
        .map(|(automation_id, _)| *automation_id)
        .collect::<std::collections::HashSet<_>>();
    let unexpected_ids = menu_nodes
        .iter()
        .map(|node| node.automation_id.as_str())
        .filter(|automation_id| !expected_ids.contains(automation_id))
        .collect::<Vec<_>>();
    if !unexpected_ids.is_empty() {
        return Err(format!(
            "accessibility menu inventory contains unreviewed section IDs: {}",
            unexpected_ids.join(", ")
        ));
    }

    let mut labels = Vec::with_capacity(REQUIRED_MENU_SECTIONS.len());
    let mut automation_ids = Vec::with_capacity(REQUIRED_MENU_SECTIONS.len());
    for (expected_id, expected_label) in REQUIRED_MENU_SECTIONS {
        let matching = menu_nodes
            .iter()
            .copied()
            .filter(|node| node.automation_id == expected_id)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(format!(
                "accessibility menu inventory requires exactly one {expected_id:?} node; observed {}; IDs: {}",
                matching.len(),
                menu_nodes
                    .iter()
                    .map(|node| node.automation_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let node = matching[0];
        if node.name != expected_label {
            return Err(format!(
                "accessibility menu node {expected_id:?} has label {:?}; expected {expected_label:?}",
                node.name
            ));
        }
        if !node.enabled || node.offscreen || node.bounds.is_none() {
            return Err(format!(
                "accessibility menu node {expected_id:?} is not interactable: enabled={}, offscreen={}, bounds={:?}",
                node.enabled, node.offscreen, node.bounds
            ));
        }
        automation_ids.push(expected_id.to_owned());
        labels.push(expected_label.to_owned());
    }
    Ok(NativeMenuEvidence {
        labels,
        automation_ids,
    })
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
                        expected_name,
                        "view:",
                        "self=",
                        "ready=",
                        "controller=",
                        "Status",
                        "Busy",
                        "Save",
                        "Reload",
                        "Connection",
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
    open_menu_section_with_acknowledgement(driver, window, menu_name, command_name, timeout)?;
    driver
        .click_named_control(window, command_name, command_kind)
        .map_err(|error| {
            format!(
                "menu leaf {menu_name:?}->{command_name:?} became visible but its single physical click failed: {error}"
            )
        })
}

fn open_menu_section_with_acknowledgement<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    section_identity: &str,
    action_identity: &str,
    timeout: Duration,
) -> Result<(), String> {
    driver
        .click_named_control(window, section_identity, NativeControlKind::Any)
        .map_err(|error| {
            format!(
                "failed the single physical click for menu section {section_identity:?}: {error}"
            )
        })?;

    let acknowledgement_deadline = Instant::now() + timeout;
    let mut last_snapshot;
    loop {
        match driver.accessibility_nodes(window) {
            Ok(nodes) => {
                let matching = nodes
                    .iter()
                    .filter(|node| {
                        node.name == action_identity || node.automation_id == action_identity
                    })
                    .collect::<Vec<_>>();
                last_snapshot = matching
                    .iter()
                    .map(|node| {
                        format!(
                            "name={:?}, automation_id={:?}, enabled={}, offscreen={}, bounds={:?}",
                            node.name,
                            node.automation_id,
                            node.enabled,
                            node.offscreen,
                            node.bounds
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                let visible = matching
                    .iter()
                    .filter(|node| !node.offscreen && node.bounds.is_some())
                    .count();
                match visible {
                    1 => return Ok(()),
                    count if count > 1 => {
                        return Err(format!(
                            "menu section {section_identity:?} exposed {count} visible matches for {action_identity:?}; snapshot: {last_snapshot}"
                        ));
                    }
                    _ => {}
                }
            }
            Err(error) => {
                last_snapshot = format!("accessibility snapshot failed: {error}");
            }
        }

        if Instant::now() >= acknowledgement_deadline {
            return Err(format!(
                "timed out waiting for one physical click on menu section {section_identity:?} to expose {action_identity:?}; the click was not redelivered because the section is a toggle; last snapshot: {last_snapshot}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn invoke_menu_action_by_id_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    section_automation_id: &str,
    action_automation_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    invoke_menu_command_with_wait(
        driver,
        window,
        section_automation_id,
        action_automation_id,
        NativeControlKind::Any,
        timeout,
    )
}

pub(super) fn invoke_menu_action_by_id_uia_only_with_wait<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    section_automation_id: &str,
    action_automation_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    invoke_named_control_with_wait(
        driver,
        window,
        section_automation_id,
        NativeControlKind::Any,
        timeout,
    )
    .map_err(|error| {
        format!(
            "failed to open menu section {section_automation_id:?} through UI Automation: {error}"
        )
    })?;

    let deadline = Instant::now() + timeout;
    loop {
        match driver.accessibility_nodes(window) {
            Ok(nodes) => {
                let visible_matches = nodes
                    .iter()
                    .filter(|node| {
                        node.automation_id == action_automation_id
                            && !node.offscreen
                            && node.bounds.is_some()
                    })
                    .count();
                match visible_matches {
                    1 => break,
                    count if count > 1 => {
                        return Err(format!(
                            "UI Automation menu section {section_automation_id:?} exposed {count} visible matches for {action_automation_id:?}"
                        ));
                    }
                    _ => {}
                }
            }
            Err(error) if Instant::now() >= deadline => {
                return Err(format!(
                    "failed to read UI Automation menu state for {action_automation_id:?}: {error}"
                ));
            }
            Err(_) => {}
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for UI Automation to expose {action_automation_id:?} from {section_automation_id:?}"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }

    invoke_named_control_with_wait(
        driver,
        window,
        action_automation_id,
        NativeControlKind::Any,
        deadline.saturating_duration_since(Instant::now()),
    )
    .map_err(|error| {
        format!(
            "failed to invoke menu action {action_automation_id:?} through UI Automation: {error}"
        )
    })
}

pub(super) fn verify_menu_action_enabled_state_by_id<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    section_automation_id: &str,
    action_automation_id: &str,
    expected_enabled: bool,
    timeout: Duration,
) -> Result<(), String> {
    open_menu_section_with_acknowledgement(
        driver,
        window,
        section_automation_id,
        action_automation_id,
        timeout,
    )?;

    let deadline = Instant::now() + timeout;
    loop {
        let last_snapshot = match driver.accessibility_nodes(window) {
            Ok(nodes) => {
                let matching = nodes
                    .iter()
                    .filter(|node| node.automation_id == action_automation_id)
                    .collect::<Vec<_>>();
                let snapshot = matching
                    .iter()
                    .map(|node| {
                        format!(
                            "name={:?}, enabled={}, offscreen={}, bounds={:?}",
                            node.name, node.enabled, node.offscreen, node.bounds
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                if matching.len() == 1
                    && matching[0].enabled == expected_enabled
                    && !matching[0].offscreen
                    && matching[0].bounds.is_some()
                {
                    driver.press_escape(window).map_err(|error| {
                        format!(
                            "verified menu action {action_automation_id:?}, but failed to dismiss section {section_automation_id:?} with Escape: {error}"
                        )
                    })?;
                    wait_for_menu_action_hidden(
                        driver,
                        window,
                        action_automation_id,
                        deadline.saturating_duration_since(Instant::now()),
                    )?;
                    // Escape leaves the menu button focused. Move focus to the stable Setup
                    // surface so the next menu click is one unambiguous request to reopen it.
                    return driver
                        .click_named_control(
                            window,
                            SETUP_SURFACE_AUTOMATION_ID,
                            NativeControlKind::Any,
                        )
                        .map_err(|error| {
                        format!(
                            "dismissed menu action {action_automation_id:?}, but failed to reset focus through {SETUP_SURFACE_AUTOMATION_ID:?}: {error}"
                        )
                    });
                }
                snapshot
            }
            Err(error) => format!("snapshot error: {error}"),
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for menu action {action_automation_id:?} to be {} after opening {section_automation_id:?}; last snapshot: {last_snapshot}",
                if expected_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_menu_action_hidden<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    action_automation_id: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        match driver.accessibility_nodes(window) {
            Ok(nodes)
                if !nodes.iter().any(|node| {
                    node.automation_id == action_automation_id
                        && !node.offscreen
                        && node.bounds.is_some()
                }) =>
            {
                return Ok(());
            }
            Ok(_) => {}
            Err(error) if Instant::now() >= deadline => {
                return Err(format!(
                    "menu action {action_automation_id:?} remained unverified after closing its section: {error}"
                ));
            }
            Err(_) => {}
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "menu action {action_automation_id:?} remained visible after dismissing its section with Escape"
            ));
        }
        thread::sleep(Duration::from_millis(50));
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

pub(super) fn invoke_menu_command_once<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    menu_name: &str,
    command_name: &str,
    timeout: Duration,
) -> Result<(), String> {
    invoke_menu_command_with_wait(
        driver,
        window,
        menu_name,
        command_name,
        NativeControlKind::Any,
        timeout,
    )
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
    invoke_menu_command_once(
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
    let acknowledgement_timeout = timeout.min(Duration::from_millis(500));
    select_top_tab_with_state_acknowledgement(
        tab_name,
        expected_name,
        timeout,
        |wait_timeout| {
            wait_for_accessible_name(driver, window, expected_name, wait_timeout).map(|_| ())
        },
        || {
            invoke_named_control_with_wait(
                driver,
                window,
                tab_name,
                NativeControlKind::Button,
                acknowledgement_timeout,
            )
        },
        || driver.click_named_control(window, tab_name, NativeControlKind::Button),
        || driver.activate_named_control_by_keyboard(window, tab_name, NativeControlKind::Button),
    )
}

fn select_top_tab_with_state_acknowledgement(
    tab_name: &str,
    expected_name: &str,
    timeout: Duration,
    mut wait_for_expected: impl FnMut(Duration) -> Result<(), String>,
    mut accessibility_invoke: impl FnMut() -> Result<(), String>,
    mut physical_click: impl FnMut() -> Result<(), String>,
    mut focused_keyboard_activation: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    let acknowledgement_timeout = timeout.min(Duration::from_millis(500));
    if wait_for_expected(acknowledgement_timeout).is_ok() {
        return Ok(());
    }

    let mut strategy_diagnostics = Vec::new();
    match accessibility_invoke() {
        Ok(()) => {
            if wait_for_expected(acknowledgement_timeout).is_ok() {
                return Ok(());
            }
            strategy_diagnostics.push(
                "accessibility invoke completed without the expected content acknowledgement"
                    .to_owned(),
            );
        }
        Err(error) => strategy_diagnostics.push(format!("accessibility invoke failed: {error}")),
    }

    match physical_click() {
        Ok(()) => {
            if wait_for_expected(acknowledgement_timeout).is_ok() {
                return Ok(());
            }
            strategy_diagnostics.push(
                "exact physical click completed without the expected content acknowledgement"
                    .to_owned(),
            );
        }
        Err(error) => strategy_diagnostics.push(format!("exact physical click failed: {error}")),
    }

    let final_wait_error = match focused_keyboard_activation() {
        Ok(()) => match wait_for_expected(timeout) {
            Ok(()) => return Ok(()),
            Err(error) => {
                strategy_diagnostics.push(
                        "focused keyboard activation completed without the expected content acknowledgement"
                            .to_owned(),
                    );
                error
            }
        },
        Err(error) => {
            strategy_diagnostics.push(format!("focused keyboard activation failed: {error}"));
            match wait_for_expected(timeout) {
                Ok(()) => return Ok(()),
                Err(error) => error,
            }
        }
    };

    Err(format!(
        "failed to activate top tab {tab_name:?}: {expected_name:?} did not appear after state-acknowledged accessibility, physical-click, and focused-keyboard strategies; {}; final state: {final_wait_error}",
        strategy_diagnostics.join("; "),
    ))
}

#[cfg(test)]
mod menu_contract_tests {
    use super::*;

    fn menu_node(automation_id: &str, name: &str) -> NativeAccessibilityNode {
        NativeAccessibilityNode {
            name: name.to_owned(),
            automation_id: automation_id.to_owned(),
            control_type: 0,
            enabled: true,
            focused: false,
            offscreen: false,
            bounds: Some([1, 2, 30, 40]),
        }
    }

    fn valid_menu_nodes() -> Vec<NativeAccessibilityNode> {
        REQUIRED_MENU_SECTIONS
            .iter()
            .map(|(automation_id, label)| menu_node(automation_id, label))
            .collect()
    }

    #[test]
    fn native_menu_contract_requires_stable_accesskit_ids_in_reviewed_order() {
        let evidence = verify_menu_contract(&valid_menu_nodes()).unwrap();
        assert_eq!(
            evidence.automation_ids,
            REQUIRED_MENU_SECTIONS
                .iter()
                .map(|(automation_id, _)| (*automation_id).to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            evidence.labels,
            REQUIRED_MENU_SECTIONS
                .iter()
                .map(|(_, label)| (*label).to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn native_menu_contract_rejects_missing_duplicate_mislabeled_and_hidden_nodes() {
        let mut missing = valid_menu_nodes();
        missing.pop();
        assert!(
            verify_menu_contract(&missing)
                .unwrap_err()
                .contains("exactly one")
        );

        let mut duplicate = valid_menu_nodes();
        duplicate.push(menu_node(FILE_MENU_AUTOMATION_ID, "File"));
        assert!(
            verify_menu_contract(&duplicate)
                .unwrap_err()
                .contains("observed 2")
        );

        let mut mislabeled = valid_menu_nodes();
        mislabeled[0].name = "Files".to_owned();
        assert!(
            verify_menu_contract(&mislabeled)
                .unwrap_err()
                .contains("expected \"File\"")
        );

        let mut hidden = valid_menu_nodes();
        hidden[0].offscreen = true;
        assert!(
            verify_menu_contract(&hidden)
                .unwrap_err()
                .contains("not interactable")
        );
    }

    #[test]
    fn native_menu_contract_rejects_unreviewed_section_ids() {
        let mut nodes = valid_menu_nodes();
        nodes.push(menu_node("menu.section.experimental", "Experimental"));
        assert!(
            verify_menu_contract(&nodes)
                .unwrap_err()
                .contains("unreviewed section IDs")
        );
    }
}

#[cfg(test)]
mod tab_activation_contract_tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn top_tab_activation_escalates_only_after_each_missing_state_acknowledgement() {
        let events = RefCell::new(Vec::new());
        let wait_count = Cell::new(0usize);

        select_top_tab_with_state_acknowledgement(
            "configuration:tab:interface-system",
            "Show OSD",
            Duration::from_secs(2),
            |_| {
                events.borrow_mut().push("wait");
                let next_wait = wait_count.get() + 1;
                wait_count.set(next_wait);
                if next_wait == 4 {
                    Ok(())
                } else {
                    Err("not visible".to_owned())
                }
            },
            || {
                events.borrow_mut().push("accessibility");
                Ok(())
            },
            || {
                events.borrow_mut().push("physical");
                Ok(())
            },
            || {
                events.borrow_mut().push("keyboard");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            [
                "wait",
                "accessibility",
                "wait",
                "physical",
                "wait",
                "keyboard",
                "wait",
            ]
        );
    }

    #[test]
    fn top_tab_activation_reports_every_failed_strategy() {
        let error = select_top_tab_with_state_acknowledgement(
            "configuration:tab:interface-system",
            "Show OSD",
            Duration::ZERO,
            |_| Err("not visible".to_owned()),
            || Err("invoke unavailable".to_owned()),
            || Err("hit test rejected".to_owned()),
            || Err("focus rejected".to_owned()),
        )
        .unwrap_err();

        assert!(error.contains("accessibility invoke failed: invoke unavailable"));
        assert!(error.contains("exact physical click failed: hit test rejected"));
        assert!(error.contains("focused keyboard activation failed: focus rejected"));
    }
}
