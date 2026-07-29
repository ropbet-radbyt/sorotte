use super::*;

impl NativeSmokeReport {
    fn render_text(&self) -> String {
        format!(
            "result=ok\nbinary={}\npid={}\nwindow_title={}\nmenu_source={}\nmenu_labels={}\nmenu_automation_ids={}\nmenu_contract={}\naccessible_name_count={}\naccessibility_contract={}\ninteraction_steps={}\ninteraction_contract={}\ncapability_outcomes={}\nclosed={}\nduration_ms={}\n",
            self.binary_path,
            self.pid,
            self.window_title,
            self.menu_source,
            self.menu_labels.join("|"),
            self.menu_automation_ids.join("|"),
            self.menu_contract,
            self.accessible_name_count,
            self.accessibility_contract,
            self.interaction_steps.join("|"),
            self.interaction_contract,
            serde_json::to_string(&self.capability_outcomes)
                .expect("native capability outcomes should serialize"),
            self.closed,
            self.duration_ms
        )
    }

    fn render_json(&self) -> String {
        let labels = self
            .menu_labels
            .iter()
            .map(|label| render_json_string(label))
            .collect::<Vec<_>>()
            .join(",");
        let interaction_steps = self
            .interaction_steps
            .iter()
            .map(|step| render_json_string(step))
            .collect::<Vec<_>>()
            .join(",");
        let menu_automation_ids = self
            .menu_automation_ids
            .iter()
            .map(|automation_id| render_json_string(automation_id))
            .collect::<Vec<_>>()
            .join(",");
        let capability_outcomes = serde_json::to_string(&self.capability_outcomes)
            .expect("native capability outcomes should serialize");
        format!(
            "{{\"result\":\"ok\",\"binary\":{},\"pid\":{},\"window_title\":{},\"menu_source\":{},\"menu_labels\":[{}],\"menu_automation_ids\":[{}],\"menu_contract\":{},\"accessible_name_count\":{},\"accessibility_contract\":{},\"interaction_steps\":[{}],\"interaction_contract\":{},\"capability_outcomes\":{},\"closed\":{},\"duration_ms\":{}}}\n",
            render_json_string(&self.binary_path),
            self.pid,
            render_json_string(&self.window_title),
            render_json_string(&self.menu_source),
            labels,
            menu_automation_ids,
            render_json_string(&self.menu_contract),
            self.accessible_name_count,
            render_json_string(&self.accessibility_contract),
            interaction_steps,
            render_json_string(&self.interaction_contract),
            capability_outcomes,
            if self.closed { "true" } else { "false" },
            self.duration_ms
        )
    }

    pub(super) fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Text => self.render_text(),
            OutputFormat::Json => self.render_json(),
        }
    }
}

pub(super) fn render_json_string(value: &str) -> String {
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

pub(super) fn render_error(error: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Text => format!("result=error\nerror={error}\n"),
        OutputFormat::Json => {
            format!(
                "{{\"result\":\"error\",\"error\":{}}}\n",
                render_json_string(error)
            )
        }
    }
}

pub(super) fn parse_timeout_ms(token: &str) -> Result<Duration, String> {
    let timeout_ms = token
        .parse::<u64>()
        .map_err(|_| format!("--timeout-ms requires a positive integer, got {token:?}"))?;
    if timeout_ms == 0 {
        return Err("--timeout-ms must be greater than zero".to_owned());
    }
    Ok(Duration::from_millis(timeout_ms))
}

pub(super) fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

pub(super) fn scenario_selected(options: &NativeSmokeOptions, scenario: &str) -> bool {
    options.scenario_filters.is_empty()
        || options
            .scenario_filters
            .iter()
            .any(|candidate| candidate == scenario)
}

pub(super) fn parse_options(args: &[String]) -> Result<NativeSmokeOptions, String> {
    let mut options = NativeSmokeOptions {
        binary_path: None,
        timeout: Duration::from_millis(10_000),
        format: OutputFormat::Text,
        keep_open: false,
        scenario_filters: Vec::new(),
    };

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--binary" => {
                if index + 1 >= args.len() {
                    return Err("--binary requires a path".to_owned());
                }
                options.binary_path = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--timeout-ms" => {
                if index + 1 >= args.len() {
                    return Err("--timeout-ms requires an integer value".to_owned());
                }
                options.timeout = parse_timeout_ms(&args[index + 1])?;
                index += 2;
            }
            "--json" => {
                options.format = OutputFormat::Json;
                index += 1;
            }
            "--text" => {
                options.format = OutputFormat::Text;
                index += 1;
            }
            "--keep-open" => {
                options.keep_open = true;
                index += 1;
            }
            "--scenario" => {
                if index + 1 >= args.len() {
                    return Err("--scenario requires a scenario name".to_owned());
                }
                options
                    .scenario_filters
                    .push(args[index + 1].to_ascii_lowercase());
                index += 2;
            }
            "--help" | "-h" => {
                return Err(native_smoke_usage().to_owned());
            }
            argument => {
                return Err(format!("unknown argument {argument:?}"));
            }
        }
    }

    Ok(options)
}

pub(super) fn native_smoke_usage() -> &'static str {
    "usage: sorotte-gui-native-smoke [--binary PATH] [--timeout-ms N] [--json|--text] [--keep-open] [--scenario NAME]"
}
