use super::*;

pub(super) fn env_trimmed(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiClientCoreChatLoopbackBootstrap {
    pub(super) username: String,
    pub(super) room: String,
}

impl GuiClientCoreChatLoopbackBootstrap {
    pub(super) fn startup_message(&self) -> String {
        format!(
            "Startup enabled client-core chat loopback via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK as {} in room {}.",
            self.username, self.room
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GuiClientCoreChatTcpBootstrap {
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) username: String,
    pub(super) room: String,
}

impl GuiClientCoreChatTcpBootstrap {
    pub(super) fn host_arg(&self) -> String {
        if self.host.contains(':') && !(self.host.starts_with('[') && self.host.ends_with(']')) {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub(super) fn startup_message_from_lookup<F>(&self, lookup: &F) -> String
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut defaults = Vec::new();
        if lookup("SYNCPLAY_CLIENT_HOST").is_none() {
            defaults.push("host=127.0.0.1");
        }
        if lookup("SYNCPLAY_CLIENT_PORT").is_none() {
            defaults.push("port=8999");
        }
        if lookup("SYNCPLAY_CLIENT_USERNAME").is_none() && lookup("SYNCPLAY_CLIENT_NAME").is_none()
        {
            defaults.push("user=gui-user");
        }
        if lookup("SYNCPLAY_CLIENT_ROOM").is_none() {
            defaults.push("room=gui-demo");
        }
        let defaults_suffix = if defaults.is_empty() {
            String::new()
        } else {
            format!(" Defaults: {}.", defaults.join(", "))
        };
        format!(
            "Startup enabled client-core chat TCP via SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP for {} as {} in room {}.",
            self.host_arg(),
            self.username,
            self.room,
        ) + &defaults_suffix
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiStartupPublicServerSource {
    FilePath(String),
    InlineEnv,
}

impl GuiStartupPublicServerSource {
    pub(super) fn from_lookup<F>(lookup: &F) -> Option<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(path) = lookup("SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            return Some(Self::FilePath(path));
        }
        lookup("SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(|_| Self::InlineEnv)
    }

    pub(super) fn startup_message(&self, server_count: usize) -> String {
        let noun = if server_count == 1 {
            "public server"
        } else {
            "public servers"
        };
        match self {
            Self::FilePath(path) => format!(
                "Startup loaded {server_count} {noun} from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS_PATH ({path})."
            ),
            Self::InlineEnv => format!(
                "Startup loaded {server_count} {noun} from SYNCPLAY_GUI_REFRESH_PUBLIC_SERVERS."
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiStartupConfigPathSource {
    Override(PathBuf),
    WorkingDirectoryExisting(PathBuf),
    ConfigRootExisting(PathBuf),
    DefaultConfigTarget(PathBuf),
}

impl GuiStartupConfigPathSource {
    pub(super) fn resolved_path(&self) -> &Path {
        match self {
            Self::Override(path)
            | Self::WorkingDirectoryExisting(path)
            | Self::ConfigRootExisting(path)
            | Self::DefaultConfigTarget(path) => path.as_path(),
        }
    }

    pub(super) fn startup_message(&self) -> String {
        let rendered_path = self.resolved_path().display();
        match self {
            Self::Override(_) => format!(
                "Startup configuration path uses SYNCPLAY_CLIENT_CONFIG_PATH ({rendered_path})."
            ),
            Self::WorkingDirectoryExisting(_) => format!(
                "Startup configuration path uses existing working-directory config ({rendered_path})."
            ),
            Self::ConfigRootExisting(_) => format!(
                "Startup configuration path uses existing config-root file ({rendered_path})."
            ),
            Self::DefaultConfigTarget(_) => format!(
                "Startup configuration path will use default config target ({rendered_path})."
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GuiStartupPlayerIpcSource {
    ClientEnv(String),
    LegacyEnv(String),
}

impl GuiStartupPlayerIpcSource {
    pub(super) fn from_lookup<F>(lookup: &F) -> Option<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        if let Some(path) = lookup("SYNCPLAY_CLIENT_MPV_IPC_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            return Some(Self::ClientEnv(path));
        }
        lookup("SYNCPLAY_MPV_IPC_PATH")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(Self::LegacyEnv)
    }

    pub(super) fn ipc_path(&self) -> &str {
        match self {
            Self::ClientEnv(path) | Self::LegacyEnv(path) => path,
        }
    }

    pub(super) fn startup_message(&self) -> String {
        match self {
            Self::ClientEnv(path) => {
                format!("Startup will try mpv JSON IPC from SYNCPLAY_CLIENT_MPV_IPC_PATH ({path}).")
            }
            Self::LegacyEnv(path) => {
                format!("Startup will try mpv JSON IPC from SYNCPLAY_MPV_IPC_PATH ({path}).")
            }
        }
    }

    pub(super) fn missing_startup_message() -> String {
        "Startup has no explicit mpv JSON IPC path. The GUI will use the saved playerPath when it points to mpv; otherwise set SYNCPLAY_CLIENT_MPV_IPC_PATH or SYNCPLAY_MPV_IPC_PATH to attach an mpv JSON IPC endpoint.".to_owned()
    }
}

pub(super) fn env_flag_enabled_lookup<F>(lookup: &F, name: &str) -> Result<bool, String>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(value) = lookup(name) else {
        return Ok(false);
    };
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "{name} must be one of: 1, true, yes, on, 0, false, no, off."
        )),
    }
}

pub(super) fn env_port_lookup<F>(lookup: &F, name: &str) -> Result<Option<u16>, String>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(value) = lookup(name) else {
        return Ok(None);
    };
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .map(Some)
        .ok_or_else(|| format!("{name} must be a valid TCP port from 1 to 65535."))
}

pub(super) fn gui_client_core_chat_tcp_bootstrap_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiClientCoreChatTcpBootstrap>, String>
where
    F: Fn(&str) -> Option<String>,
{
    if !env_flag_enabled_lookup(&lookup, "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_TCP")? {
        return Ok(None);
    }

    Ok(Some(GuiClientCoreChatTcpBootstrap {
        host: lookup("SYNCPLAY_CLIENT_HOST").unwrap_or_else(|| "127.0.0.1".to_owned()),
        port: env_port_lookup(&lookup, "SYNCPLAY_CLIENT_PORT")?.unwrap_or(8999),
        username: lookup("SYNCPLAY_CLIENT_USERNAME")
            .or_else(|| lookup("SYNCPLAY_CLIENT_NAME"))
            .unwrap_or_else(|| "gui-user".to_owned()),
        room: lookup("SYNCPLAY_CLIENT_ROOM").unwrap_or_else(|| "gui-demo".to_owned()),
    }))
}

pub(super) fn gui_client_core_chat_loopback_bootstrap_from_lookup<F>(
    lookup: F,
) -> Result<Option<GuiClientCoreChatLoopbackBootstrap>, String>
where
    F: Fn(&str) -> Option<String>,
{
    if !env_flag_enabled_lookup(&lookup, "SYNCPLAY_GUI_ENABLE_CLIENT_CORE_CHAT_LOOPBACK")? {
        return Ok(None);
    }

    Ok(Some(GuiClientCoreChatLoopbackBootstrap {
        username: lookup("SYNCPLAY_CLIENT_USERNAME")
            .or_else(|| lookup("SYNCPLAY_CLIENT_NAME"))
            .unwrap_or_else(|| "gui-user".to_owned()),
        room: lookup("SYNCPLAY_CLIENT_ROOM").unwrap_or_else(|| "gui-demo".to_owned()),
    }))
}
