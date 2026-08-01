use sorotte_secret::SecretValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostArgumentError {
    EmptyHost,
    EmptyPort,
    NonNumericPort,
    PortOutOfRange,
    MalformedBracketedIpv6,
}

impl std::fmt::Display for HostArgumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyHost => "host is empty",
            Self::EmptyPort => "port is empty",
            Self::NonNumericPort => "port is not numeric",
            Self::PortOutOfRange => "port must be between 1 and 65535",
            Self::MalformedBracketedIpv6 => "bracketed IPv6 endpoint is malformed",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LegacyClientArgumentIssue {
    UnknownOption {
        name: String,
        attached_value_present: bool,
    },
    MissingValue {
        name: String,
    },
    InvalidHost {
        name: String,
        error: HostArgumentError,
    },
}

impl LegacyClientArgumentIssue {
    pub(crate) fn unknown_option(argument: &str) -> Self {
        let (name, attached_value_present) = argument
            .split_once('=')
            .map_or((argument, false), |(name, _)| (name, true));
        Self::UnknownOption {
            name: name.to_owned(),
            attached_value_present,
        }
    }

    pub(crate) fn unknown_short_option(name: char, attached_value_present: bool) -> Self {
        Self::UnknownOption {
            name: format!("-{name}"),
            attached_value_present,
        }
    }

    pub(crate) fn missing_value(name: &str) -> Self {
        Self::MissingValue {
            name: name.to_owned(),
        }
    }

    pub(crate) fn invalid_host(name: &str, error: HostArgumentError) -> Self {
        Self::InvalidHost {
            name: name.to_owned(),
            error,
        }
    }

    pub(crate) fn is_host_argument(&self) -> bool {
        matches!(self, Self::InvalidHost { .. })
    }

    fn unknown_option_has_attached_value(&self) -> bool {
        match self {
            Self::UnknownOption {
                attached_value_present,
                ..
            } => *attached_value_present,
            _ => false,
        }
    }

    pub(crate) fn diagnostic_fragment(&self) -> String {
        match self {
            Self::UnknownOption { name, .. } => {
                if self.unknown_option_has_attached_value() {
                    format!("{name}={}", sorotte_secret::REDACTED_SECRET)
                } else {
                    name.clone()
                }
            }
            Self::MissingValue { name } => name.clone(),
            Self::InvalidHost { name, error } => format!("{name} ({error})"),
        }
    }

    #[cfg(test)]
    pub(crate) fn matches_rejected_token(&self, token: &str) -> bool {
        match self {
            Self::UnknownOption {
                name,
                attached_value_present,
            } => token.split_once('=').map_or_else(
                || name == token && !attached_value_present,
                |(expected_name, _)| name == expected_name && *attached_value_present,
            ),
            Self::MissingValue { name } => name == token,
            Self::InvalidHost { .. } => false,
        }
    }
}

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
    pub(crate) unknown_options: Vec<LegacyClientArgumentIssue>,
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
