use super::GuiPersistedConfigRuntimeOwner;
use super::feature_slices::GuiRuntimeInput;
use super::runtime_bridge::GuiQueuedRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;

impl GuiQueuedRuntimeOwner for GuiPersistedConfigRuntimeOwner {
    fn input_changed(&mut self, _handle: &GuiQueuedRuntimeBridgeHandle, input: &GuiRuntimeInput) {
        self.update_runtime.reconcile(input.updates());
        self.legacy_projection = Some(input.to_compatibility_projection());
    }

    fn poll(&mut self, handle: &GuiQueuedRuntimeBridgeHandle) {
        self.poll_cached_runtime(handle);
    }
}
