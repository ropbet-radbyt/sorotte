mod catalog;
mod cli;
mod custom_flows;
mod external_script;

use super::semantic_driver::GuiSemanticScenario;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiSemanticScenarioReport {
    pub scenario: String,
    pub view: String,
    pub modal: String,
    pub pending: String,
    pub widgets: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiSemanticScenarioDescriptor {
    pub name: &'static str,
    pub description: &'static str,
    pub script: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiSemanticOutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiSemanticScenarioSource {
    Named(String),
    ScriptPath(String),
    InlineScript(String),
}

impl GuiSemanticScenarioReport {
    fn render_text(&self) -> String {
        format!(
            "result=ok\nscenario={}\nview={}\nmodal={}\npending={}\nwidgets={}\n",
            self.scenario, self.view, self.modal, self.pending, self.widgets
        )
    }

    fn render_json(&self) -> String {
        format!(
            "{{\"result\":\"ok\",\"scenario\":{},\"view\":{},\"modal\":{},\"pending\":{},\"widgets\":{}}}\n",
            render_json_string(&self.scenario),
            render_json_string(&self.view),
            render_json_string(&self.modal),
            render_json_string(&self.pending),
            self.widgets
        )
    }

    pub fn render(&self, format: GuiSemanticOutputFormat) -> String {
        match format {
            GuiSemanticOutputFormat::Text => self.render_text(),
            GuiSemanticOutputFormat::Json => self.render_json(),
        }
    }
}

fn render_json_string(value: &str) -> String {
    let mut rendered = String::with_capacity(value.len() + 2);
    rendered.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => rendered.push_str("\\\\"),
            '"' => rendered.push_str("\\\""),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            _ => rendered.push(ch),
        }
    }
    rendered.push('"');
    rendered
}

#[cfg(test)]
fn normalize_script_line_endings(script: &str) -> String {
    catalog::normalize_script_line_endings(script)
}

pub(crate) fn gui_semantic_scenario_script(name: &str) -> Option<&'static str> {
    catalog::gui_semantic_scenario_script(name)
}

pub(crate) fn gui_semantic_scenario_descriptors() -> Vec<GuiSemanticScenarioDescriptor> {
    catalog::gui_semantic_scenario_descriptors()
}

pub fn run_syncplay_gui_semantic_scenario_catalog(format: GuiSemanticOutputFormat) -> String {
    catalog::run_syncplay_gui_semantic_scenario_catalog(format)
}

pub(crate) fn gui_semantic_scenario_names() -> &'static [&'static str] {
    catalog::gui_semantic_scenario_names()
}

pub(super) fn gui_semantic_scenario_named(name: &str) -> Option<GuiSemanticScenario> {
    catalog::gui_semantic_scenario_named(name)
}

#[cfg(test)]
pub(super) fn gui_semantic_scenario_name_from_lookup<F>(lookup: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    cli::gui_semantic_scenario_name_from_lookup(lookup)
}

#[cfg(test)]
pub(super) fn gui_semantic_output_format_from_lookup<F>(
    lookup: F,
) -> Result<GuiSemanticOutputFormat, String>
where
    F: Fn(&str) -> Option<String>,
{
    cli::gui_semantic_output_format_from_lookup(lookup)
}

fn run_gui_semantic_scenario(
    scenario: GuiSemanticScenario,
) -> Result<GuiSemanticScenarioReport, String> {
    let driver = scenario.run()?;
    Ok(GuiSemanticScenarioReport {
        scenario: scenario.name().to_owned(),
        view: driver.active_view_label().to_owned(),
        modal: driver.active_modal_label().to_owned(),
        pending: driver.pending_operation_label().to_owned(),
        widgets: driver.widget_count(),
    })
}

pub(super) fn run_gui_semantic_scenario_named(
    name: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    match name {
        "live-python-peer-connect-flow" => {
            custom_flows::run_gui_semantic_live_python_peer_connect_flow()
        }
        "live-python-peer-controlled-room-flow" => {
            custom_flows::run_gui_semantic_live_python_peer_controlled_room_flow()
        }
        "persistence-reset-flow" => custom_flows::run_gui_semantic_persistence_reset_flow(),
        "detached-runtime-ownership-flow" => {
            custom_flows::run_gui_semantic_detached_runtime_ownership_flow()
        }
        _ => {
            let scenario = gui_semantic_scenario_named(name).ok_or_else(|| {
                format!(
                    "unknown semantic scenario {name:?}. Available: {}",
                    gui_semantic_scenario_names().join(", ")
                )
            })?;
            run_gui_semantic_scenario(scenario)
        }
    }
}

fn read_semantic_script_from_path(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read semantic scenario script {path:?}: {error}"))
}

pub fn run_syncplay_gui_semantic_report_from_named_with_append_script_path(
    name: &str,
    append_script_path: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    if matches!(
        name,
        "detached-runtime-ownership-flow"
            | "live-python-peer-connect-flow"
            | "live-python-peer-controlled-room-flow"
    ) {
        return Err(format!(
            "--append-script does not support custom semantic scenario {name:?}"
        ));
    }
    let base_script = gui_semantic_scenario_script(name).ok_or_else(|| {
        format!(
            "unknown semantic scenario {name:?}. Available: {}",
            gui_semantic_scenario_names().join(", ")
        )
    })?;
    let append_script = read_semantic_script_from_path(append_script_path)?;
    let mut combined_script = String::with_capacity(base_script.len() + append_script.len() + 1);
    combined_script.push_str(base_script);
    if !base_script.ends_with('\n') {
        combined_script.push('\n');
    }
    combined_script.push_str(&append_script);
    let mut report = external_script::run_gui_semantic_external_script(
        &format!("{name:?}+{append_script_path:?}"),
        &combined_script,
    )?;
    if report.scenario == "external-script" {
        report.scenario = name.to_owned();
    }
    Ok(report)
}

pub fn run_syncplay_gui_semantic_report(
    source: GuiSemanticScenarioSource,
) -> Result<GuiSemanticScenarioReport, String> {
    match source {
        GuiSemanticScenarioSource::Named(name) => run_gui_semantic_scenario_named(&name),
        GuiSemanticScenarioSource::ScriptPath(path) => {
            let script = read_semantic_script_from_path(&path)?;
            let mut report =
                external_script::run_gui_semantic_external_script(&format!("{path:?}"), &script)?;
            if report.scenario == "external-script" {
                report.scenario = path;
            }
            Ok(report)
        }
        GuiSemanticScenarioSource::InlineScript(script) => {
            external_script::run_gui_semantic_external_script("inline-script", &script)
        }
    }
}

pub fn run_syncplay_gui_semantic_output(
    source: GuiSemanticScenarioSource,
    format: GuiSemanticOutputFormat,
) -> Result<String, String> {
    run_syncplay_gui_semantic_report(source).map(|report| report.render(format))
}

pub fn run_syncplay_gui_semantic_report_from_script(
    script: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::InlineScript(script.to_owned()))
}

pub fn run_syncplay_gui_semantic_report_from_script_path(
    path: &str,
) -> Result<GuiSemanticScenarioReport, String> {
    run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::ScriptPath(path.to_owned()))
}

pub(crate) fn run_syncplay_gui_semantic_cli_from_args<I, S>(
    args: I,
) -> Result<Option<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    cli::run_syncplay_gui_semantic_cli_from_args_impl(args)
}

pub(super) fn run_gui_semantic_scenario_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiSemanticScenarioReport>, String>
where
    F: Fn(&str) -> Option<String>,
{
    cli::run_gui_semantic_scenario_from_lookup(lookup)
}

pub(super) fn run_gui_semantic_scenario_output_from_lookup<F>(
    lookup: F,
) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Option<String>,
{
    cli::run_gui_semantic_scenario_output_from_lookup(lookup)
}

pub(crate) fn run_syncplay_gui_semantic_report_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiSemanticScenarioReport>, String>
where
    F: Fn(&str) -> Option<String>,
{
    run_gui_semantic_scenario_from_lookup(lookup)
}

pub(crate) fn run_syncplay_gui_semantic_cli_from_lookup<F>(
    lookup: F,
) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Option<String>,
{
    run_gui_semantic_scenario_output_from_lookup(lookup)
}

pub(crate) fn run_syncplay_gui_semantic_report_from_env()
-> Result<Option<GuiSemanticScenarioReport>, String> {
    run_syncplay_gui_semantic_report_from_lookup(cli::semantic_env_trimmed)
}

pub(crate) fn run_syncplay_gui_semantic_cli_from_env() -> Result<Option<String>, String> {
    run_syncplay_gui_semantic_cli_from_lookup(cli::semantic_env_trimmed)
}

#[cfg(test)]
mod tests;
