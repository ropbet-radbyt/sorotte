//! Display-condition fixtures own only generated protocol input and native assertions.
//! They use a disposable TCP peer and test player; no saved user session is involved.
use super::*;

fn member_name(index: usize) -> String {
    format!("viewer-{index:03}-{}", "long-label-\u{754c}".repeat(3))
}

fn playlist_name(index: usize) -> String {
    format!(
        "episode-{index:03}-{}.mkv",
        "long-title-\u{754c}".repeat(12)
    )
}

pub(super) fn content_stress_frames() -> Vec<String> {
    let mut members = serde_json::Map::new();
    for index in 0..127 {
        members.insert(
            member_name(index),
            serde_json::json!({
                "file":{"name": playlist_name(index), "duration": 1440.0, "size": 1024},
                "isReady": true
            }),
        );
    }
    members.insert(
        CONFIG_USERNAME_VALUE.to_owned(),
        serde_json::json!({"isReady": false}),
    );
    let files = (0..256).map(playlist_name).collect::<Vec<_>>();
    vec![
        serde_json::json!({"Hello": {
            "username": CONFIG_USERNAME_VALUE, "room":{"name": CONFIG_ROOM_VALUE},
            "version":"1.7.5", "features":{"chat":true,"readiness":true,"sharedPlaylists":true}
        }})
        .to_string(),
        serde_json::json!({"List":{CONFIG_ROOM_VALUE: members}}).to_string(),
        serde_json::json!({"Set":{"playlistChange":{"files":files,"user":"fixture"}}}).to_string(),
    ]
}

fn content_node_is_visible(node: &NativeAccessibilityNode, window: [i32; 4]) -> bool {
    !node.offscreen
        && node.enabled
        && node.bounds.is_some_and(|[left, top, right, bottom]| {
            left >= window[0]
                && top >= window[1]
                && right <= window[2]
                && bottom <= window[3]
                && right > left
                && bottom > top
        })
}

fn content_scroll_delta(target: Option<[i32; 4]>, window: [i32; 4], down: bool) -> i32 {
    let Some(target) = target else {
        return if down { -14_400 } else { 14_400 };
    };
    // Use measured target distance to cover long content without a full UIA
    // snapshot for every wheel notch. Re-observation chooses direction again,
    // so a stacked panel or an overshoot cannot leave the search at one edge.
    let distance = ((i64::from(window[1]) + i64::from(window[3]))
        - (i64::from(target[1]) + i64::from(target[3])))
        / 2;
    (distance.signum() * distance.abs().clamp(120, 14_400)) as i32
}

fn scroll_to_content<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    container: &str,
    label: &str,
    _viewport_height: i32,
    timeout: Duration,
    down: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let playlist = container == "main-window:playlist-surface";
    let mut steps = 0;
    loop {
        let read_started = Instant::now();
        let nodes = driver.accessibility_nodes(window)?;
        let mut window_bounds = nodes
            .iter()
            .find(|node| node.control_type == 50_032)
            .and_then(|node| node.bounds)
            .ok_or("content snapshot has no native window bounds")?;
        if let Some(bounds) = nodes
            .iter()
            .find(|node| node.automation_id == "menu.section.file")
            .and_then(|node| node.bounds)
        {
            window_bounds[1] = window_bounds[1].max(bounds[3] + 1);
        }
        if let Some(bounds) = nodes
            .iter()
            .find(|node| node.automation_id == "main-window-root")
            .and_then(|node| node.bounds)
        {
            window_bounds[0] = window_bounds[0].max(bounds[2] + 1);
        }
        window_bounds[2] -= 10;
        window_bounds[3] -= 10;
        if nodes.iter().any(|node| {
            node.name.contains(label)
                // The room also exposes file labels. Only a playlist's actual
                // button row can satisfy the playlist endpoint assertion.
                && (!playlist || node.control_type == 50_000)
                && content_node_is_visible(node, window_bounds)
        }) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{container} content {label:?} remained unreachable by scrolling"
            ));
        }
        // These custom painted panels have no UIA control of their own. Wheel
        // over an actual visible content node, whose center is on screen,
        // rather than assuming a semantic-tree panel is an input control.
        let anchor = nodes
            .iter()
            .filter(|node| {
                content_node_is_visible(node, window_bounds)
                    && !node.name.is_empty()
                    && node.bounds.is_some_and(|[left, top, right, bottom]| {
                        left >= window_bounds[0]
                            && right > left
                            && top >= window_bounds[1]
                            && bottom <= window_bounds[3]
                            && bottom > top
                    })
            })
            .max_by_key(|node| {
                let matching_content = if playlist {
                    node.control_type == 50_000 && node.name.starts_with("episode-")
                } else {
                    node.name.starts_with("viewer-")
                };
                // A label can be reported inside the HWND while its center
                // falls in the scroll area's clipped bottom margin. Prefer
                // the middle of the content viewport for physical input.
                let center_distance = node.bounds.map_or(i64::MAX, |bounds| {
                    ((i64::from(bounds[1]) + i64::from(bounds[3]))
                        - (i64::from(window_bounds[1]) + i64::from(window_bounds[3])))
                    .abs()
                });
                (matching_content, std::cmp::Reverse(center_distance))
            })
            .ok_or_else(|| {
                format!("{container} has no on-screen content anchor for physical scrolling")
            })?;
        let target = nodes
            .iter()
            .find(|node| {
                node.name.contains(label)
                    && (!playlist || node.control_type == 50_000)
                    && node.bounds.is_some_and(|[left, _, right, _]| {
                        left >= window_bounds[0] && right <= window_bounds[2]
                    })
            })
            .and_then(|node| node.bounds);
        let delta = content_scroll_delta(target, window_bounds, down);
        steps += 1;
        eprintln!(
            "native content scroll {steps}: target={label:?} snapshot_ms={} delta={delta} viewport={window_bounds:?} target_bounds={target:?} anchor={:?} anchor_bounds={:?}",
            read_started.elapsed().as_millis(),
            anchor.name,
            anchor.bounds
        );
        driver.scroll_named_content_page(window, anchor, delta)?;
    }
}

pub(super) fn exercise_content_stress<D: NativeGuiDriver>(
    driver: &D,
    window: D::WindowHandle,
    viewport_height: i32,
    timeout: Duration,
) -> Result<(), String> {
    // The constrained room layout stacks roster and playlist in one outer
    // scroll area. Walk down through both before reversing direction.
    for (container, label, down) in [
        ("main-window:connection", member_name(0), false),
        ("main-window:connection", member_name(126), true),
        ("main-window:playlist-surface", playlist_name(0), true),
        ("main-window:playlist-surface", playlist_name(255), true),
        ("main-window:playlist-surface", playlist_name(0), false),
        ("main-window:connection", member_name(0), false),
    ] {
        scroll_to_content(
            driver,
            window,
            container,
            &label,
            viewport_height,
            timeout,
            down,
        )?;
    }
    scroll_to_content(
        driver,
        window,
        "main-window:playlist-surface",
        &playlist_name(255),
        viewport_height,
        timeout,
        true,
    )?;
    driver.activate_named_control_by_keyboard(
        window,
        &playlist_name(255),
        NativeControlKind::Button,
    )?;
    let nodes = driver.accessibility_nodes(window)?;
    if !nodes
        .iter()
        .any(|node| node.focused && node.name.contains(&playlist_name(255)) && !node.offscreen)
    {
        return Err("final playlist row did not retain labeled keyboard focus".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_scroll_uses_observed_distance_and_reverses_after_crossing_a_target() {
        let window = [23, 32, 941, 806];
        assert_eq!(
            content_scroll_delta(Some([300, 30_000, 700, 30_021]), window, false),
            -14_400
        );
        assert!(content_scroll_delta(Some([300, 1000, 700, 1021]), window, false) < 0);
        assert!(content_scroll_delta(Some([300, -100, 700, -79]), window, true) > 0);
        assert_eq!(content_scroll_delta(None, window, true), -14_400);
    }

    #[test]
    fn content_visibility_rejects_off_window_duplicates_and_clipped_endpoints() {
        let mut node = NativeAccessibilityNode {
            name: member_name(126),
            automation_id: String::new(),
            control_type: 50_020,
            enabled: true,
            focused: false,
            offscreen: false,
            bounds: Some([300, 160, 700, 190]),
        };
        let window = [23, 32, 941, 806];
        assert!(content_node_is_visible(&node, window));
        for bounds in [
            [3972, 160, 4408, 190],
            [300, 795, 700, 820],
            [300, -50, 700, -20],
            [300, 160, 300, 190],
        ] {
            node.bounds = Some(bounds);
            assert!(!content_node_is_visible(&node, window));
        }
        node.bounds = Some([300, 160, 700, 190]);
        node.offscreen = true;
        assert!(!content_node_is_visible(&node, window));
    }

    #[test]
    fn stress_fixture_has_complete_legal_frames_and_long_content() {
        let frames = content_stress_frames();
        for line in &frames {
            assert!(line.len() <= sorotte_protocol::SOROTTE_MAX_PROTOCOL_LINE_BYTES);
            sorotte_protocol::decode_message_line(line)
                .expect("real decoder accepts generated fixture");
        }
        let roster: serde_json::Value = serde_json::from_str(&frames[1]).unwrap();
        assert_eq!(
            roster["List"][CONFIG_ROOM_VALUE].as_object().unwrap().len(),
            128
        );
        let playlist: serde_json::Value = serde_json::from_str(&frames[2]).unwrap();
        assert_eq!(
            playlist["Set"]["playlistChange"]["files"]
                .as_array()
                .unwrap()
                .len(),
            256
        );
        assert!(playlist_name(255).len() > 150);
    }

    #[test]
    fn display_options_reject_invalid_values_and_wrong_native_dpi() {
        for value in ["NaN", "inf", "0", "-1", "3.1"] {
            assert!(parse_visual_suite_options(&["--ui-scale".into(), value.into()]).is_err());
        }
        let options = parse_visual_suite_options(&[
            "--ui-scale".into(),
            "1.5".into(),
            "--expected-native-dpi".into(),
            "144".into(),
            "--theme".into(),
            "dark".into(),
            "--scenario".into(),
            "room.content-stress".into(),
        ])
        .unwrap();
        assert_eq!(options.display.ui_scale, 1.5);
        assert_eq!(options.display.theme, Some("dark"));
        assert!(options.display.check_native_dpi(144).is_ok());
        assert!(options.display.check_native_dpi(96).is_err());
    }
}
