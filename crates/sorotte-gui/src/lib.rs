mod app;

pub use app::run_sorotte_gui;

#[cfg(feature = "gui-semantic-smoke")]
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

    pub fn run_sorotte_gui_semantic_report_from_env()
    -> Result<Option<GuiSemanticScenarioReport>, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_report_from_env()
    }

    pub fn run_sorotte_gui_semantic_report_from_lookup<F>(
        lookup: F,
    ) -> Result<Option<GuiSemanticScenarioReport>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        super::app::semantic_smoke::run_sorotte_gui_semantic_report_from_lookup(lookup)
    }

    pub fn run_sorotte_gui_semantic_report_from_script(
        script: &str,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_report_from_script(script)
    }

    pub fn run_sorotte_gui_semantic_report_from_script_path(
        path: &str,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_report_from_script_path(path)
    }

    pub fn run_sorotte_gui_semantic_report_from_named_with_append_script_path(
        name: &str,
        append_script_path: &str,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_report_from_named_with_append_script_path(
            name,
            append_script_path,
        )
    }

    pub fn run_sorotte_gui_semantic_report(
        source: GuiSemanticScenarioSource,
    ) -> Result<GuiSemanticScenarioReport, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_report(source)
    }

    pub fn run_sorotte_gui_semantic_output(
        source: GuiSemanticScenarioSource,
        format: GuiSemanticOutputFormat,
    ) -> Result<String, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_output(source, format)
    }

    pub fn run_sorotte_gui_semantic_scenario_catalog(format: GuiSemanticOutputFormat) -> String {
        super::app::semantic_smoke::run_sorotte_gui_semantic_scenario_catalog(format)
    }

    pub fn run_sorotte_gui_semantic_cli_from_args<I, S>(args: I) -> Result<Option<String>, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        super::app::semantic_smoke::run_sorotte_gui_semantic_cli_from_args(args)
    }

    pub fn run_sorotte_gui_semantic_cli_from_env() -> Result<Option<String>, String> {
        super::app::semantic_smoke::run_sorotte_gui_semantic_cli_from_env()
    }

    pub fn run_sorotte_gui_semantic_cli_from_lookup<F>(lookup: F) -> Result<Option<String>, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        super::app::semantic_smoke::run_sorotte_gui_semantic_cli_from_lookup(lookup)
    }
}
