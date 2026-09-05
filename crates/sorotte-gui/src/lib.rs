mod app;

use sorotte_lifecycle_evidence::{
    Disposition, ProcessRole, TargetKind, TransitionObservation, Trigger, emit_global,
};

#[derive(Clone, Copy)]
pub(crate) struct GuiLifecycleOrigin {
    process_role: ProcessRole,
    subject: &'static str,
}

impl GuiLifecycleOrigin {
    pub(crate) const fn new(process_role: ProcessRole, subject: &'static str) -> Self {
        Self {
            process_role,
            subject,
        }
    }
}

pub(crate) fn emit_gui_lifecycle_transition(
    origin: GuiLifecycleOrigin,
    transition: &'static str,
    machine: &'static str,
    target: TargetKind,
    trigger: Trigger,
    disposition: Disposition,
    identities: &[(&'static str, u64)],
) {
    let mut observation =
        TransitionObservation::new(origin.process_role, origin.subject, machine, transition)
            .target(target)
            .triggered_by(trigger)
            .authority("gui-pending", "gui-applied")
            .effect("lifecycle-transition", "lifecycle-transition")
            .disposition(disposition);
    for (name, value) in identities.iter().copied().filter(|(_, value)| *value > 0) {
        observation = observation.identity(name, value);
    }
    let _ = emit_global(observation);
}

pub use app::run_sorotte_gui;

#[cfg(feature = "gui-semantic-smoke")]
pub mod semantic_smoke {
    pub use super::app::semantic_driver::GuiProjectionMeasurement;

    /// Measures repeated shell snapshot application and widget projection on one initialized
    /// headless driver. Parsing, startup and native rendering are outside the timed pumps.
    pub fn measure_projection(
        script: &str,
        pumps: usize,
    ) -> Result<GuiProjectionMeasurement, String> {
        super::app::semantic_driver::measure_projection(script, pumps)
    }

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
