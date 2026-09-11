use super::super::StoredClientSettings;
use super::{GuiSemanticDriver, GuiSemanticStep};

#[derive(Debug, Clone, PartialEq)]
pub(in crate::app) struct GuiSemanticScenario {
    name: &'static str,
    initial_settings: StoredClientSettings,
    steps: Vec<GuiSemanticStep>,
}

impl GuiSemanticScenario {
    fn new(
        name: &'static str,
        initial_settings: StoredClientSettings,
        steps: Vec<GuiSemanticStep>,
    ) -> Self {
        Self {
            name,
            initial_settings,
            steps,
        }
    }

    pub(in crate::app) fn from_script(
        name: &'static str,
        initial_settings: StoredClientSettings,
        script: &str,
    ) -> Result<Self, String> {
        Ok(Self::new(
            name,
            initial_settings,
            GuiSemanticStep::parse_script(script)?,
        ))
    }

    pub(in crate::app) fn name(&self) -> &'static str {
        self.name
    }

    fn initial_settings(&self) -> &StoredClientSettings {
        &self.initial_settings
    }

    fn steps(&self) -> &[GuiSemanticStep] {
        &self.steps
    }

    pub(in crate::app) fn run(&self) -> Result<GuiSemanticDriver, String> {
        let mut driver = GuiSemanticDriver::from_stored_settings(self.initial_settings());
        driver.run_steps(self.steps())?;
        Ok(driver)
    }
}
