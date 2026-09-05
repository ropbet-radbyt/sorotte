use super::GuiPersistedConfigRuntimeOwner;
use super::feature_slices::GuiRuntimeInput;
use super::runtime_bridge::GuiQueuedRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
#[cfg(any(
    feature = "gui-semantic-smoke",
    all(test, feature = "live-python-interop")
))]
use super::shell_state::SorotteGuiShellAppState;

#[cfg(any(
    feature = "gui-semantic-smoke",
    all(test, feature = "live-python-interop")
))]
impl GuiPersistedConfigRuntimeOwner {
    /// Compatibility entry point for direct, single-threaded semantic adapters.
    /// Production's threaded bridge submits compact input only when it changes.
    pub(in crate::app) fn pump_compatibility_state(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        state: &SorotteGuiShellAppState,
    ) {
        let input = GuiRuntimeInput::from_shell(state);
        <Self as GuiQueuedRuntimeOwner>::input_changed(self, handle, &input);
        <Self as GuiQueuedRuntimeOwner>::poll(self, handle);
    }
}

impl GuiQueuedRuntimeOwner for GuiPersistedConfigRuntimeOwner {
    fn register_owned_processes(
        &self,
        scope: &sorotte_player_mpv::managed_process::ManagedMpvShutdownScope,
    ) -> Result<(), String> {
        if let Some(process) = &self.managed_mpv_process {
            process.register_shutdown_scope(scope)?;
        }
        Ok(())
    }

    fn input_changed(&mut self, _handle: &GuiQueuedRuntimeBridgeHandle, input: &GuiRuntimeInput) {
        self.update_runtime.reconcile(input.updates());
        self.legacy_projection = Some(input.to_compatibility_projection());
    }

    fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        if handle.threaded_runtime_shutdown_requested() {
            return;
        }
        self.poll_cached_runtime(handle);
    }
}
