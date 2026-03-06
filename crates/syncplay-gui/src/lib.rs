#[allow(unused_imports)]
#[path = "main.rs"]
mod app;

pub mod semantic_smoke {
    pub use super::app::semantic_smoke::{
        GuiSemanticOutputFormat, GuiSemanticScenarioDescriptor, GuiSemanticScenarioReport,
        GuiSemanticScenarioSource,
    };

    pub fn scenario_names() -> &'static [&'static str] {
        super::app::semantic_smoke::gui_semantic_scenario_names()
    }

    pub fn scenario_script(name: &str) -> Option<&'static str> {
        super::app::semantic_smoke::gui_semantic_scenario_script(name)
    }

    pub fn scenario_descriptors() -> Vec<GuiSemanticScenarioDescriptor> {
        super::app::semantic_smoke::gui_semantic_scenario_descriptors()
    }

    pub fn run_syncplay_gui_semantic_report_from_env()
    -> Result<Option<GuiSemanticScenarioReport>, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_report_from_env()
    }

    pub fn run_syncplay_gui_semantic_report_from_lookup<F>(
        lookup: F,
    ) -> Result<Option<GuiSemanticScenarioReport>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        super::app::semantic_smoke::run_syncplay_gui_semantic_report_from_lookup(lookup)
    }

    pub fn run_syncplay_gui_semantic_report_from_script(
        script: &str,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_report_from_script(script)
    }

    pub fn run_syncplay_gui_semantic_report_from_script_path(
        path: &str,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_report_from_script_path(path)
    }

    pub fn run_syncplay_gui_semantic_report_from_named_with_append_script_path(
        name: &str,
        append_script_path: &str,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_report_from_named_with_append_script_path(
            name,
            append_script_path,
        )
    }

    pub fn run_syncplay_gui_semantic_report(
        source: GuiSemanticScenarioSource,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_report(source)
    }

    pub fn run_syncplay_gui_semantic_output(
        source: GuiSemanticScenarioSource,
        format: GuiSemanticOutputFormat,
    ) -> Result<String, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_output(source, format)
    }

    pub fn run_syncplay_gui_semantic_scenario_catalog(format: GuiSemanticOutputFormat) -> String {
        super::app::semantic_smoke::run_syncplay_gui_semantic_scenario_catalog(format)
    }

    pub fn run_syncplay_gui_semantic_cli_from_args<I, S>(args: I) -> Result<Option<String>, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        super::app::semantic_smoke::run_syncplay_gui_semantic_cli_from_args(args)
    }

    pub fn run_syncplay_gui_semantic_cli_from_env() -> Result<Option<String>, String> {
        super::app::semantic_smoke::run_syncplay_gui_semantic_cli_from_env()
    }

    pub fn run_syncplay_gui_semantic_cli_from_lookup<F>(lookup: F) -> Result<Option<String>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        super::app::semantic_smoke::run_syncplay_gui_semantic_cli_from_lookup(lookup)
    }
}

pub fn semantic_smoke_scenario_names() -> &'static [&'static str] {
    semantic_smoke::scenario_names()
}

pub fn semantic_smoke_scenario_script(name: &str) -> Option<&'static str> {
    semantic_smoke::scenario_script(name)
}

pub fn semantic_smoke_scenario_descriptors() -> Vec<semantic_smoke::GuiSemanticScenarioDescriptor> {
    semantic_smoke::scenario_descriptors()
}

pub fn run_syncplay_gui_semantic_report_from_env()
-> Result<Option<semantic_smoke::GuiSemanticScenarioReport>, String> {
    semantic_smoke::run_syncplay_gui_semantic_report_from_env()
}

pub fn run_syncplay_gui_semantic_report_from_lookup<F>(
    lookup: F,
) -> Result<Option<semantic_smoke::GuiSemanticScenarioReport>, String>
where
    F: Fn(&str) -> Option<String>,
{
    semantic_smoke::run_syncplay_gui_semantic_report_from_lookup(lookup)
}

pub fn run_syncplay_gui_semantic_report_from_script(
    script: &str,
) -> Result<semantic_smoke::GuiSemanticScenarioReport, String> {
    semantic_smoke::run_syncplay_gui_semantic_report_from_script(script)
}

pub fn run_syncplay_gui_semantic_report_from_script_path(
    path: &str,
) -> Result<semantic_smoke::GuiSemanticScenarioReport, String> {
    semantic_smoke::run_syncplay_gui_semantic_report_from_script_path(path)
}

pub fn run_syncplay_gui_semantic_report_from_named_with_append_script_path(
    name: &str,
    append_script_path: &str,
) -> Result<semantic_smoke::GuiSemanticScenarioReport, String> {
    semantic_smoke::run_syncplay_gui_semantic_report_from_named_with_append_script_path(
        name,
        append_script_path,
    )
}

pub fn run_syncplay_gui_semantic_report(
    source: semantic_smoke::GuiSemanticScenarioSource,
) -> Result<semantic_smoke::GuiSemanticScenarioReport, String> {
    semantic_smoke::run_syncplay_gui_semantic_report(source)
}

pub fn run_syncplay_gui_semantic_output(
    source: semantic_smoke::GuiSemanticScenarioSource,
    format: semantic_smoke::GuiSemanticOutputFormat,
) -> Result<String, String> {
    semantic_smoke::run_syncplay_gui_semantic_output(source, format)
}

pub fn run_syncplay_gui_semantic_scenario_catalog(
    format: semantic_smoke::GuiSemanticOutputFormat,
) -> String {
    semantic_smoke::run_syncplay_gui_semantic_scenario_catalog(format)
}

pub fn run_syncplay_gui_semantic_cli_from_args<I, S>(args: I) -> Result<Option<String>, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    semantic_smoke::run_syncplay_gui_semantic_cli_from_args(args)
}

pub fn run_syncplay_gui_semantic_cli_from_env() -> Result<Option<String>, String> {
    semantic_smoke::run_syncplay_gui_semantic_cli_from_env()
}

pub fn run_syncplay_gui_semantic_cli_from_lookup<F>(lookup: F) -> Result<Option<String>, String>
where
    F: Fn(&str) -> Option<String>,
{
    semantic_smoke::run_syncplay_gui_semantic_cli_from_lookup(lookup)
}
