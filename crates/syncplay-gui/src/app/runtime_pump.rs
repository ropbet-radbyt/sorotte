use super::GuiPersistedConfigRuntimeOwner;
use super::runtime_bridge::GuiQueuedRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::SyncplayGuiShellAppState;

impl GuiQueuedRuntimeOwner for GuiPersistedConfigRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SyncplayGuiShellAppState) {
        self.pump_runtime(handle, state);
    }
}
