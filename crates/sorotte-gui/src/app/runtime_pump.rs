use super::GuiPersistedConfigRuntimeOwner;
use super::feature_slices::GuiRuntimeInput;
use super::runtime_bridge::GuiQueuedRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::SorotteGuiShellAppState;

impl GuiQueuedRuntimeOwner for GuiPersistedConfigRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SorotteGuiShellAppState) {
        self.pump_runtime(handle, state);
    }

    fn pump_runtime_input(
        &mut self,
        handle: &GuiQueuedRuntimeBridgeHandle,
        input: &GuiRuntimeInput,
    ) {
        self.pump_runtime_projection(handle, input.to_compatibility_projection());
    }
}
