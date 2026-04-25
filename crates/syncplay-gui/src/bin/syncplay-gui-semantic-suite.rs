const DEFAULT_PUBLIC_SERVER_LIST_RESPONSE: &str =
    "[['Alpha', 'alpha.example:8999'], ['Beta', 'beta.example:9000']]";
const DEFAULT_UPDATE_CHECK_RESPONSE: &str =
    r#"{"version-status":"uptodate","version-message":"Syncplay is up to date."}"#;

fn install_semantic_remote_response_defaults() {
    set_semantic_env_default(
        "SYNCPLAY_GUI_PUBLIC_SERVER_LIST_RESPONSE",
        DEFAULT_PUBLIC_SERVER_LIST_RESPONSE,
    );
    set_semantic_env_default(
        "SYNCPLAY_GUI_UPDATE_CHECK_RESPONSE",
        DEFAULT_UPDATE_CHECK_RESPONSE,
    );
}

fn set_semantic_env_default(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        set_semantic_env_var_before_threads(key, value);
    }
}

fn set_semantic_env_var_before_threads(key: &str, value: &str) {
    // SAFETY: The semantic suite installs deterministic defaults before scenario runtime threads
    // start, and only when the caller has not already provided an environment override.
    unsafe {
        std::env::set_var(key, value);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SuiteOutputFormat {
    Text,
    Json,
}

struct SuiteOptions {
    format: SuiteOutputFormat,
    list_only: bool,
    scenarios: Vec<String>,
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

fn parse_options(args: &[String]) -> Result<SuiteOptions, String> {
    let mut format = SuiteOutputFormat::Text;
    let mut list_only = false;
    let mut scenarios = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                format = SuiteOutputFormat::Json;
                index += 1;
            }
            "--text" => {
                format = SuiteOutputFormat::Text;
                index += 1;
            }
            "--list" => {
                list_only = true;
                index += 1;
            }
            "--scenario" => {
                if index + 1 >= args.len() {
                    return Err("--scenario requires a scenario name".to_owned());
                }
                scenarios.push(args[index + 1].clone());
                index += 2;
            }
            "--help" | "-h" => {
                return Err(
                    "usage: syncplay-gui-semantic-suite [--list] [--scenario NAME ...] [--json|--text]"
                        .to_owned(),
                );
            }
            argument => {
                return Err(format!("unknown argument {argument:?}"));
            }
        }
    }
    if list_only && !scenarios.is_empty() {
        return Err("--list cannot be combined with --scenario".to_owned());
    }
    Ok(SuiteOptions {
        format,
        list_only,
        scenarios,
    })
}

fn render_list(names: &[&str], format: SuiteOutputFormat) -> String {
    match format {
        SuiteOutputFormat::Text => format!("{}\n", names.join("\n")),
        SuiteOutputFormat::Json => {
            let entries = names
                .iter()
                .map(|name| render_json_string(name))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{\"result\":\"ok\",\"scenarios\":[{entries}]}}\n")
        }
    }
}

fn render_summary_text(
    reports: &[syncplay_gui::semantic_smoke::GuiSemanticScenarioReport],
    failures: &[(String, String)],
) -> String {
    let mut rendered = format!(
        "suite=syncplay-gui-semantic-suite\ntotal={}\npassed={}\nfailed={}\n",
        reports.len() + failures.len(),
        reports.len(),
        failures.len()
    );
    for report in reports {
        rendered.push('\n');
        rendered.push_str(&format!("[scenario:{}]\n", report.scenario));
        rendered
            .push_str(&report.render(syncplay_gui::semantic_smoke::GuiSemanticOutputFormat::Text));
    }
    for (name, error) in failures {
        rendered.push('\n');
        rendered.push_str(&format!("[scenario:{name}]\nresult=error\nerror={error}\n"));
    }
    rendered
}

fn render_summary_json(
    reports: &[syncplay_gui::semantic_smoke::GuiSemanticScenarioReport],
    failures: &[(String, String)],
) -> String {
    let report_entries = reports
        .iter()
        .map(|report| {
            format!(
                "{{\"result\":\"ok\",\"scenario\":{},\"view\":{},\"modal\":{},\"pending\":{},\"widgets\":{}}}",
                render_json_string(&report.scenario),
                render_json_string(&report.view),
                render_json_string(&report.modal),
                render_json_string(&report.pending),
                report.widgets
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let failure_entries = failures
        .iter()
        .map(|(name, error)| {
            format!(
                "{{\"scenario\":{},\"error\":{}}}",
                render_json_string(name),
                render_json_string(error)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"result\":{},\"total\":{},\"passed\":{},\"failed\":{},\"reports\":[{}],\"errors\":[{}]}}\n",
        if failures.is_empty() {
            "\"ok\""
        } else {
            "\"error\""
        },
        reports.len() + failures.len(),
        reports.len(),
        failures.len(),
        report_entries,
        failure_entries,
    )
}

fn main() {
    install_semantic_remote_response_defaults();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("syncplay-gui-semantic-suite failed: {error}");
            std::process::exit(2);
        }
    };

    let builtins = syncplay_gui::semantic_smoke::scenario_names();
    if options.list_only {
        print!("{}", render_list(builtins, options.format));
        return;
    }

    let scenario_names = if options.scenarios.is_empty() {
        builtins
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    } else {
        options.scenarios
    };

    let mut reports = Vec::new();
    let mut failures = Vec::new();
    for scenario_name in scenario_names {
        if !builtins.contains(&scenario_name.as_str()) {
            failures.push((
                scenario_name.clone(),
                format!(
                    "unknown semantic scenario {:?}. Available: {}",
                    scenario_name,
                    builtins.join(", ")
                ),
            ));
            continue;
        }
        match syncplay_gui::semantic_smoke::run_syncplay_gui_semantic_report(
            syncplay_gui::semantic_smoke::GuiSemanticScenarioSource::Named(scenario_name.clone()),
        ) {
            Ok(report) => reports.push(report),
            Err(error) => failures.push((scenario_name, error)),
        }
    }

    let output = match options.format {
        SuiteOutputFormat::Text => render_summary_text(&reports, &failures),
        SuiteOutputFormat::Json => render_summary_json(&reports, &failures),
    };
    print!("{output}");
    if !failures.is_empty() {
        std::process::exit(1);
    }
}
