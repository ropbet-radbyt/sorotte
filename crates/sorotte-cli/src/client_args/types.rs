use sorotte_secret::SecretValue;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct LegacyClientArgOverrides {
    pub(crate) connect_requested: bool,
    pub(crate) no_store: bool,
    pub(crate) debug_requested: bool,
    pub(crate) force_gui_prompt_requested: bool,
    pub(crate) no_gui_requested: bool,
    pub(crate) clear_gui_data_requested: bool,
    pub(crate) config_path: Option<String>,
    pub(crate) config_root: Option<String>,
    pub(crate) language: Option<String>,
    pub(crate) player_path: Option<String>,
    pub(crate) file: Option<String>,
    pub(crate) player_args: Vec<String>,
    pub(crate) load_playlist_from_file: Option<String>,
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) username: Option<String>,
    pub(crate) room: Option<String>,
    pub(crate) controlled_room_password_override: Option<SecretValue>,
    pub(crate) show_help: bool,
    pub(crate) show_version: bool,
    pub(crate) unknown_options: Vec<String>,
}

impl std::fmt::Debug for LegacyClientArgOverrides {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LegacyClientArgOverrides")
            .field("connect_requested", &self.connect_requested)
            .field("no_store", &self.no_store)
            .field("debug_requested", &self.debug_requested)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("room_configured", &self.room.is_some())
            .field(
                "controlled_room_password_override",
                &self.controlled_room_password_override,
            )
            .field("show_help", &self.show_help)
            .field("show_version", &self.show_version)
            .finish_non_exhaustive()
    }
}

impl LegacyClientArgOverrides {
    pub(crate) fn should_connect_client(&self) -> bool {
        self.connect_requested
            || self.host.is_some()
            || self.port.is_some()
            || self.username.is_some()
            || self.room.is_some()
            || self.controlled_room_password_override.is_some()
            || self.player_path.is_some()
            || self.file.is_some()
            || !self.player_args.is_empty()
    }

    pub(crate) fn should_halt_for_legacy_force_gui_prompt_compatibility(&self) -> bool {
        self.force_gui_prompt_requested && !self.no_gui_requested
    }
}
