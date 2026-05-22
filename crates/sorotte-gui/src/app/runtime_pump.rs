use super::GuiPersistedConfigRuntimeOwner;
use super::runtime_bridge::GuiQueuedRuntimeOwner;
use super::runtime_queue::GuiQueuedRuntimeBridgeHandle;
use super::shell_state::SorotteGuiShellAppState;

impl GuiQueuedRuntimeOwner for GuiPersistedConfigRuntimeOwner {
    fn pump(&mut self, handle: &GuiQueuedRuntimeBridgeHandle, state: &SorotteGuiShellAppState) {
        self.pump_runtime(handle, state);
    }
}
