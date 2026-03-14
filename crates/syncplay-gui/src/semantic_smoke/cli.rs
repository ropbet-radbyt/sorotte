use super::{GuiSemanticOutputFormat, GuiSemanticScenarioReport, GuiSemanticScenarioSource};

fn parse_gui_semantic_output_format(
    value: Option<&str>,
) -> Result<GuiSemanticOutputFormat, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("text") => Ok(GuiSemanticOutputFormat::Text),
        Some("json") => Ok(GuiSemanticOutputFormat::Json),
        Some(value) => Err(format!(
            "unknown semantic output format {value:?}. Expected 'text' or 'json'"
        )),
    }
}

pub(super) fn gui_semantic_scenario_name_from_lookup<F>(lookup: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup("SYNCPLAY_GUI_SEMANTIC_SCENARIO")
}

pub(super) fn gui_semantic_scenario_script_path_from_lookup<F>(lookup: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup("SYNCPLAY_GUI_SEMANTIC_SCENARIO_PATH")
}

pub(super) fn gui_semantic_output_format_from_lookup<F>(
    lookup: F,
) -> Result<GuiSemanticOutputFormat, String>
where
    F: Fn(&str) -> Option<String>,
{
    parse_gui_semantic_output_format(lookup("SYNCPLAY_GUI_SEMANTIC_OUTPUT").as_deref())
}

pub(super) fn run_syncplay_gui_semantic_cli_from_args_impl<I, S>(
    args: I,
) -> Result<Option<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut selection: Option<GuiSemanticScenarioSource> = None;
    let mut format = GuiSemanticOutputFormat::Text;
    let mut list = false;
    let mut printed_script: Option<String> = None;
    let mut describe_scenarios = false;
    let mut append_script_path: Option<String> = None;
    let mut args = args.into_iter();
    while let Some(argument) = args.next() {
        match argument.as_ref() {
            "--scenario" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--scenario requires a scenario name".to_owned())?;
                if selection.is_some() {
                    return Err(
                        "semantic smoke accepts only one of --scenario, --script, or --inline-script"
                            .to_owned(),
                    );
                }
                selection = Some(GuiSemanticScenarioSource::Named(value.as_ref().to_owned()));
            }
            "--script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--script requires a script path".to_owned())?;
                if selection.is_some() {
                    return Err(
                        "semantic smoke accepts only one of --scenario, --script, or --inline-script"
                            .to_owned(),
                    );
                }
                selection = Some(GuiSemanticScenarioSource::ScriptPath(
                    value.as_ref().to_owned(),
                ));
            }
            "--inline-script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--inline-script requires script text".to_owned())?;
                if selection.is_some() {
                    return Err(
                        "semantic smoke accepts only one of --scenario, --script, or --inline-script"
                            .to_owned(),
                    );
                }
                selection = Some(GuiSemanticScenarioSource::InlineScript(
                    value.as_ref().to_owned(),
                ));
            }
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--format requires 'text' or 'json'".to_owned())?;
                format = parse_gui_semantic_output_format(Some(value.as_ref()))?;
            }
            "--list" => {
                list = true;
            }
            "--print-script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--print-script requires a scenario name".to_owned())?;
                if printed_script.is_some() {
                    return Err("--print-script can only be provided once".to_owned());
                }
                printed_script = Some(value.as_ref().to_owned());
            }
            "--describe-scenarios" => {
                describe_scenarios = true;
            }
            "--append-script" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--append-script requires a script path".to_owned())?;
                if append_script_path.is_some() {
                    return Err("--append-script can only be provided once".to_owned());
                }
                append_script_path = Some(value.as_ref().to_owned());
            }
            other => {
                return Err(format!("unknown semantic smoke argument {other:?}"));
            }
        }
    }

    if list {
        if selection.is_some()
            || printed_script.is_some()
            || describe_scenarios
            || append_script_path.is_some()
        {
            return Err(
                "--list cannot be combined with --scenario, --script, --inline-script, --print-script, --describe-scenarios, or --append-script"
                    .to_owned(),
            );
        }
        return Ok(Some(format!(
            "{}\n",
            super::gui_semantic_scenario_names().join("\n")
        )));
    }

    if let Some(name) = printed_script {
        if selection.is_some() || describe_scenarios || append_script_path.is_some() {
            return Err(
                "--print-script cannot be combined with --scenario, --script, --inline-script, --describe-scenarios, or --append-script"
                    .to_owned(),
            );
        }
        let Some(script) = super::gui_semantic_scenario_script(&name) else {
            return Err(format!(
                "unknown semantic scenario {name:?}. Available: {}",
                super::gui_semantic_scenario_names().join(", ")
            ));
        };
        return Ok(Some(script.to_owned()));
    }

    if describe_scenarios {
        if selection.is_some() || append_script_path.is_some() {
            return Err(
                "--describe-scenarios cannot be combined with --scenario, --script, --inline-script, or --append-script"
                    .to_owned(),
            );
        }
        return Ok(Some(super::run_syncplay_gui_semantic_scenario_catalog(
            format,
        )));
    }

    if let Some(append_script_path) = append_script_path {
        return match selection {
            Some(GuiSemanticScenarioSource::Named(name)) => {
                super::run_syncplay_gui_semantic_report_from_named_with_append_script_path(
                    &name,
                    &append_script_path,
                )
                .map(|report| Some(report.render(format)))
            }
            Some(_) => Err("--append-script currently supports only --scenario NAME".to_owned()),
            None => Err("--append-script requires --scenario NAME".to_owned()),
        };
    }

    selection
        .map(|selection| super::run_syncplay_gui_semantic_output(selection, format))
        .transpose()
}

pub(super) fn run_gui_semantic_scenario_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiSemanticScenarioReport>, String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(path) = gui_semantic_scenario_script_path_from_lookup(&lookup) {
        return super::run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::ScriptPath(
            path,
        ))
        .map(Some);
    }
    let Some(name) = gui_semantic_scenario_name_from_lookup(lookup) else {
        return Ok(None);
    };
    super::run_syncplay_gui_semantic_report(GuiSemanticScenarioSource::Named(name)).map(Some)
}

pub(super) fn run_gui_semantic_scenario_output_from_lookup<F>(
    lookup: F,
) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Option<String>,
{
    let format = gui_semantic_output_format_from_lookup(&lookup)?;
    run_gui_semantic_scenario_from_lookup(lookup)
        .map(|report| report.map(|report| report.render(format)))
}

pub(super) fn semantic_env_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
