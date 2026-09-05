//! Headless projection measurements share the semantic driver and production shell reducer.
use std::time::Instant;

use super::{GuiSemanticDriver, GuiSemanticStep};
use crate::app::StoredClientSettingsMvp;

fn user_rows(node: &crate::app::GuiWidgetNode) -> usize {
    usize::from(
        node.id
            .strip_prefix("main-window:user:")
            .is_some_and(|suffix| suffix.parse::<usize>().is_ok()),
    ) + node.children.iter().map(user_rows).sum::<usize>()
}

#[derive(Debug, serde::Serialize)]
#[non_exhaustive]
pub struct GuiProjectionMeasurement {
    pub pump_nanoseconds: Vec<u64>,
    pub widgets: usize,
    pub projected_users: usize,
    pub projected_playlist_items: usize,
}

pub(crate) fn measure_projection(
    script: &str,
    pumps: usize,
) -> Result<GuiProjectionMeasurement, String> {
    if !(1..=10_000).contains(&pumps) {
        return Err("projection pumps must be in 1..=10000".to_owned());
    }
    let steps = GuiSemanticStep::parse_script(script)?;
    let [GuiSemanticStep::ApplyMainWindowRuntimeSnapshot(snapshot)] = steps.as_slice() else {
        return Err("projection workload requires exactly one runtime snapshot".to_owned());
    };
    let mut driver = GuiSemanticDriver::from_stored_settings(&StoredClientSettingsMvp::default());
    driver.run_steps(&steps)?;
    let mut result = GuiProjectionMeasurement {
        pump_nanoseconds: Vec::with_capacity(pumps),
        widgets: 0,
        projected_users: snapshot.users.len(),
        projected_playlist_items: snapshot.playlist.len(),
    };
    for _ in 0..pumps {
        let started = Instant::now();
        driver.run_steps(&steps)?;
        result.widgets = std::hint::black_box(driver.widget_count());
        result
            .pump_nanoseconds
            .push(started.elapsed().as_nanos() as u64);
    }
    let tree = driver.shell_tree();
    result.projected_users = user_rows(&tree);
    result.projected_playlist_items = tree
        .find("main-window:playlist")
        .ok_or("playlist projection missing")?
        .children
        .len();
    if result.projected_users != snapshot.users.len()
        || result.projected_playlist_items != snapshot.playlist.len()
    {
        return Err("GUI projection lost fixture rows".to_owned());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measurements_require_projection_and_preserve_all_rows() {
        assert!(measure_projection("", 2).is_err());
        assert!(measure_projection("activate\tmain-window.toggle-ready", 2).is_err());
        let script = "apply-main-window-runtime\troom\ttrue\tfalse\tfalse\ttrue\ttrue\tfalse\ttrue\tself,true,true,false|bob,false,false,true\tone.mkv|two.mkv\tsystem>connected";
        assert!(measure_projection(script, 0).is_err());
        let result = measure_projection(script, 3).unwrap();
        assert_eq!(result.pump_nanoseconds.len(), 3);
        assert_eq!(result.projected_users, 2);
        assert_eq!(result.projected_playlist_items, 2);
        assert!(result.widgets > 0);
    }
}
