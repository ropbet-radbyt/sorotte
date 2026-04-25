use super::*;

impl GuiEframeNativeHost {
    pub(in crate::app) fn native_options() -> eframe::NativeOptions {
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("Syncplay GUI")
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([960.0, 640.0])
                .with_drag_and_drop(true),
            ..Default::default()
        }
    }

    pub(in crate::app) fn with_runtime_and_pump(
        runtime: Box<dyn GuiNativeRuntimeBridge>,
        runtime_pump: Box<dyn GuiNativeRuntimePump>,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            runtime_pump: Some(runtime_pump),
            runtime_repaint_handle: None,
        }
    }

    pub(in crate::app) fn with_runtime(runtime: Box<dyn GuiNativeRuntimeBridge>) -> Self {
        Self::with_runtime_and_pump(runtime, Box::<GuiNoopRuntimePump>::default())
    }

    pub(in crate::app) fn with_queued_runtime_owner<TOwner>(
        show_manual_pending_controls: bool,
        owner: TOwner,
    ) -> Self
    where
        TOwner: GuiQueuedRuntimeOwner + Send + 'static,
    {
        let (runtime, handle) =
            GuiQueuedRuntimeBridge::new_with_manual_pending_controls(show_manual_pending_controls);
        let repaint_handle = handle.clone();
        let mut host = Self::with_runtime_and_pump(
            Box::new(runtime),
            Box::new(GuiThreadedRuntimeOwnerPump::new(handle, owner)),
        );
        host.runtime_repaint_handle = Some(repaint_handle);
        host
    }

    pub(in crate::app) fn with_queued_preview_runtime_for_config_path(
        config_path: Option<PathBuf>,
    ) -> Self {
        Self::with_queued_runtime_owner(
            false,
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path),
        )
    }

    pub(in crate::app) fn with_queued_preview_runtime() -> Self {
        Self::with_queued_preview_runtime_for_config_path(None)
    }

    pub(in crate::app) fn with_client_core_chat_session_for_config_path(
        username: impl Into<String>,
        room: impl Into<String>,
        config_path: Option<PathBuf>,
    ) -> Result<(Self, GuiQueuedSessionTransportHandle), String> {
        let (owner, session_transport) =
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path)
                .with_client_core_chat_session_runtime(username, room)?;
        Ok((
            Self::with_queued_runtime_owner(false, owner),
            session_transport,
        ))
    }

    #[allow(dead_code)]
    pub(in crate::app) fn with_client_core_chat_session(
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<(Self, GuiQueuedSessionTransportHandle), String> {
        Self::with_client_core_chat_session_for_config_path(username, room, None)
    }

    pub(in crate::app) fn with_client_core_chat_loopback_session_for_config_path(
        username: impl Into<String>,
        room: impl Into<String>,
        config_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let owner =
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path)
                .with_client_core_chat_loopback_session_runtime(username, room)?;
        Ok(Self::with_queued_runtime_owner(false, owner))
    }

    #[allow(dead_code)]
    pub(in crate::app) fn with_client_core_chat_loopback_session(
        username: impl Into<String>,
        room: impl Into<String>,
    ) -> Result<Self, String> {
        Self::with_client_core_chat_loopback_session_for_config_path(username, room, None)
    }

    pub(in crate::app) fn with_client_core_chat_tcp_session_for_config_path(
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
        config_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let owner =
            GuiPersistedConfigRuntimeOwner::with_config_path_and_startup_player(config_path)
                .with_client_core_chat_tcp_session_runtime(username, room, host_arg)?;
        Ok(Self::with_queued_runtime_owner(false, owner))
    }

    #[allow(dead_code)]
    pub(in crate::app) fn with_client_core_chat_tcp_session(
        username: impl Into<String>,
        room: impl Into<String>,
        host_arg: impl AsRef<str>,
    ) -> Result<Self, String> {
        Self::with_client_core_chat_tcp_session_for_config_path(username, room, host_arg, None)
    }

    #[allow(dead_code)]
    pub(in crate::app) fn with_queued_runtime() -> (Self, GuiQueuedRuntimeBridgeHandle) {
        let (runtime, handle) = GuiQueuedRuntimeBridge::new();
        let mut host = Self::with_runtime(Box::new(runtime));
        host.runtime_repaint_handle = Some(handle.clone());
        (host, handle)
    }
}

impl Default for GuiEframeNativeHost {
    fn default() -> Self {
        Self::with_queued_preview_runtime()
    }
}

impl GuiAppHost for GuiEframeNativeHost {
    type Output = eframe::Result<()>;

    fn render(&mut self, state: SyncplayGuiShellAppState) -> Self::Output {
        let runtime = self
            .runtime
            .take()
            .unwrap_or_else(|| Box::<GuiPreviewRuntimeBridge>::default());
        let runtime_pump = self
            .runtime_pump
            .take()
            .unwrap_or_else(|| Box::<GuiNoopRuntimePump>::default());
        let runtime_repaint_handle = self.runtime_repaint_handle.take();
        eframe::run_native(
            "Syncplay GUI",
            Self::native_options(),
            Box::new(move |creation_context| {
                Ok(Box::new(GuiNativeApp::new(
                    creation_context,
                    state,
                    runtime,
                    runtime_pump,
                    runtime_repaint_handle,
                )))
            }),
        )
    }
}
