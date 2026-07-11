use super::feature_slices::GuiRuntimeInput;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::SorotteGuiShellAppState;
use super::{GuiPersistedConfigRuntimeOwner, GuiQueuedRuntimeOwner};

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
    fn input_changed(&mut self, _handle: &GuiQueuedRuntimeBridgeHandle, input: &GuiRuntimeInput) {
        self.update_runtime.reconcile(input.updates());
        self.legacy_projection = Some(input.to_compatibility_projection());
    }

    fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        self.poll_cached_runtime(handle);
    }
}
