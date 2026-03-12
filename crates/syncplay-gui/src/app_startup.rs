use super::*;

pub(super) fn gui_startup_settings_from_lookup_with<F, R, C, I, L>(
    lookup: F,
    read_to_string: R,
    current_dir: C,
    is_file: I,
    load_settings_at_path: L,
) -> Result<StoredClientSettingsMvp, String>
where
    F: Fn(&str) -> Option<String>,
    R: Fn(&str) -> Result<String, String>,
    C: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
    L: Fn(&Path) -> Result<Option<StoredClientSettingsMvp>, String>,
{
    let config_path_source = resolve_syncplay_gui_config_path_source_legacy_compatible_with(
        &lookup,
        current_dir,
        is_file,
    );
    let mut settings = match config_path_source.as_ref() {
        Some(source) => load_settings_at_path(source.resolved_path())?.unwrap_or_default(),
        None => StoredClientSettingsMvp::default(),
    };
    if let Some(bootstrap) = gui_client_core_chat_loopback_bootstrap_from_lookup(&lookup)? {
        settings.username = Some(bootstrap.username);
        settings.room = Some(bootstrap.room);
        settings.chat_input_enabled = Some(true);
        settings.chat_output_enabled = Some(true);
    } else if let Some(bootstrap) = gui_client_core_chat_tcp_bootstrap_from_lookup(&lookup)? {
        settings.host = Some(bootstrap.host);
        settings.port = Some(bootstrap.port);
        settings.username = Some(bootstrap.username);
        settings.room = Some(bootstrap.room);
        settings.chat_input_enabled = Some(true);
        settings.chat_output_enabled = Some(true);
    }
    if let Some(public_servers) =
        GuiClientCoreChatSessionRuntimeAdapter::refreshed_public_server_rows_from_sources(
            &lookup,
            &read_to_string,
        )?
    {
        settings.public_servers = Some(public_servers);
    }
    Ok(settings)
}

pub(super) fn gui_startup_settings_from_lookup<F, R>(
    lookup: F,
    read_to_string: R,
) -> Result<StoredClientSettingsMvp, String>
where
    F: Fn(&str) -> Option<String>,
    R: Fn(&str) -> Result<String, String>,
{
    gui_startup_settings_from_lookup_with(
        lookup,
        read_to_string,
        || env::current_dir().ok(),
        Path::is_file,
        |path| {
            load_syncplay_ini_stored_client_settings_mvp_from_path(path)
                .map_err(|error| error.to_string())
        },
    )
}

pub(super) fn gui_startup_host_and_settings()
-> Result<(GuiEframeNativeHost, StoredClientSettingsMvp), String> {
    let config_path = resolve_syncplay_gui_config_path_legacy_compatible();
    let settings = gui_startup_settings_from_lookup(env_trimmed, |path| {
        std::fs::read_to_string(path).map_err(|error| error.to_string())
    })?;
    if let Some(bootstrap) = gui_client_core_chat_loopback_bootstrap_from_lookup(env_trimmed)? {
        let host = GuiEframeNativeHost::with_client_core_chat_loopback_session_for_config_path(
            bootstrap.username,
            bootstrap.room,
            config_path,
        )?;
        return Ok((host, settings));
    }
    let Some(bootstrap) = gui_client_core_chat_tcp_bootstrap_from_lookup(env_trimmed)? else {
        return Ok((
            GuiEframeNativeHost::with_queued_preview_runtime_for_config_path(config_path),
            settings,
        ));
    };

    let host = GuiEframeNativeHost::with_client_core_chat_tcp_session_for_config_path(
        bootstrap.username.clone(),
        bootstrap.room.clone(),
        bootstrap.host_arg(),
        config_path,
    )?;
    Ok((host, settings))
}

fn syncplay_config_names_legacy_compatible() -> [&'static str; 2] {
    [".syncplay", "syncplay.ini"]
}

fn syncplay_gui_config_path_override_from_lookup<F>(lookup: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    lookup("SYNCPLAY_CLIENT_CONFIG_PATH")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(super) fn explicit_mpv_ipc_path_from_lookup<F>(lookup: &F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    GuiStartupPlayerIpcSource::from_lookup(lookup).map(|source| source.ipc_path().to_owned())
}

fn default_syncplay_gui_config_root_legacy_compatible_from_lookup<F>(lookup: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if cfg!(windows) {
        return lookup("APPDATA")
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
    }
    if let Some(xdg_config_home) = lookup("XDG_CONFIG_HOME")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(xdg_config_home));
    }
    lookup("HOME")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
}

pub(super) fn resolve_syncplay_gui_config_path_source_legacy_compatible_with<F, C, I>(
    lookup: &F,
    current_dir: C,
    is_file: I,
) -> Option<GuiStartupConfigPathSource>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
{
    if let Some(path) = syncplay_gui_config_path_override_from_lookup(lookup) {
        return Some(GuiStartupConfigPathSource::Override(path));
    }
    if let Some(cwd) = current_dir() {
        for name in syncplay_config_names_legacy_compatible() {
            let candidate = cwd.join(name);
            if is_file(&candidate) {
                return Some(GuiStartupConfigPathSource::WorkingDirectoryExisting(
                    candidate,
                ));
            }
        }
    }
    let root = default_syncplay_gui_config_root_legacy_compatible_from_lookup(lookup)?;
    for name in syncplay_config_names_legacy_compatible() {
        let candidate = root.join(name);
        if is_file(&candidate) {
            return Some(GuiStartupConfigPathSource::ConfigRootExisting(candidate));
        }
    }
    Some(GuiStartupConfigPathSource::DefaultConfigTarget(
        root.join("syncplay.ini"),
    ))
}

pub(super) fn resolve_syncplay_gui_config_path_legacy_compatible() -> Option<PathBuf> {
    resolve_syncplay_gui_config_path_source_legacy_compatible_with(
        &env_trimmed,
        || env::current_dir().ok(),
        Path::is_file,
    )
    .map(|source| source.resolved_path().to_path_buf())
}

fn syncplay_gui_qsettings_root_from_config_path_source(
    source: Option<GuiStartupConfigPathSource>,
) -> Option<PathBuf> {
    source.and_then(|source| source.resolved_path().parent().map(Path::to_path_buf))
}

fn syncplay_gui_qsettings_root_from_lookup<F>(lookup: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    syncplay_gui_qsettings_root_from_config_path_source(
        resolve_syncplay_gui_config_path_source_legacy_compatible_with(
            lookup,
            || env::current_dir().ok(),
            Path::is_file,
        ),
    )
}

pub(super) fn syncplay_gui_qsettings_root_from_env() -> Option<PathBuf> {
    syncplay_gui_qsettings_root_from_lookup(&env_trimmed)
}

pub(super) fn load_gui_ui_state_from_lookup<F>(
    lookup: &F,
) -> Result<Option<GuiPersistedUiState>, String>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(root) = syncplay_gui_qsettings_root_from_lookup(lookup) else {
        return Ok(None);
    };
    load_gui_ui_state_from_root(&root)
}

pub(super) fn gui_startup_actions_from_lookup<F>(
    lookup: F,
    settings: &StoredClientSettingsMvp,
) -> Vec<GuiShellAction>
where
    F: Fn(&str) -> Option<String>,
{
    let config_path_source = resolve_syncplay_gui_config_path_source_legacy_compatible_with(
        &lookup,
        || env::current_dir().ok(),
        Path::is_file,
    );
    let mut messages = gui_startup_messages_from_lookup_and_config_path_source(
        &lookup,
        settings,
        config_path_source,
    );
    if GuiStartupPlayerIpcSource::from_lookup(&lookup).is_none() {
        messages.push(GuiStartupPlayerIpcSource::missing_startup_message());
    }
    let mut actions = gui_startup_actions_from_messages(messages);
    if !cfg!(test) {
        actions.extend(gui_startup_remote_actions(settings));
    }
    actions
}

#[cfg(test)]
pub(super) fn gui_startup_actions_from_lookup_and_config_path_source<F>(
    lookup: F,
    settings: &StoredClientSettingsMvp,
    config_path_source: Option<GuiStartupConfigPathSource>,
) -> Vec<GuiShellAction>
where
    F: Fn(&str) -> Option<String>,
{
    gui_startup_actions_from_messages(gui_startup_messages_from_lookup_and_config_path_source(
        &lookup,
        settings,
        config_path_source,
    ))
}

fn gui_startup_messages_from_lookup_and_config_path_source<F>(
    lookup: &F,
    settings: &StoredClientSettingsMvp,
    config_path_source: Option<GuiStartupConfigPathSource>,
) -> Vec<String>
where
    F: Fn(&str) -> Option<String>,
{
    let mut messages = Vec::new();
    if let Some(bootstrap) = gui_client_core_chat_loopback_bootstrap_from_lookup(lookup)
        .ok()
        .flatten()
    {
        messages.push(bootstrap.startup_message());
    } else if let Some(bootstrap) = gui_client_core_chat_tcp_bootstrap_from_lookup(lookup)
        .ok()
        .flatten()
    {
        messages.push(bootstrap.startup_message_from_lookup(lookup));
    }
    if let Some(source) = GuiStartupPublicServerSource::from_lookup(lookup) {
        messages.push(source.startup_message(settings.public_servers.as_ref().map_or(0, Vec::len)));
    }
    if let Some(source) = GuiStartupPlayerIpcSource::from_lookup(lookup) {
        messages.push(source.startup_message());
    }
    if let Some(source) = config_path_source {
        messages.push(source.startup_message());
    }
    messages
}

fn gui_startup_actions_from_messages(messages: Vec<String>) -> Vec<GuiShellAction> {
    if messages.is_empty() {
        return Vec::new();
    }
    let summary_message = if messages.len() == 1 {
        messages[0].clone()
    } else {
        format!(
            "Startup summary: {} startup notices active. Check system chat for details.",
            messages.len()
        )
    };
    let mut actions = messages
        .into_iter()
        .map(GuiShellAction::AnnounceSystemChatEvent)
        .collect::<Vec<_>>();
    actions.push(GuiShellAction::PushTransientNotification {
        level: GuiTransientNotificationLevel::Info,
        message: summary_message,
    });
    actions
}

fn gui_startup_remote_actions(settings: &StoredClientSettingsMvp) -> Vec<GuiShellAction> {
    gui_startup_remote_actions_with_fetchers(
        settings,
        SystemTime::now(),
        |language| remote_services::check_for_updates(Some(language), false),
        |language| remote_services::fetch_public_servers(Some(language)),
    )
}

pub(super) fn gui_startup_remote_actions_with_fetchers<FUpdate, FPublicServers>(
    settings: &StoredClientSettingsMvp,
    now: SystemTime,
    fetch_update_check: FUpdate,
    fetch_public_servers: FPublicServers,
) -> Vec<GuiShellAction>
where
    FUpdate: Fn(&str) -> remote_services::LegacyUpdateCheckResult,
    FPublicServers: Fn(&str) -> Result<Vec<(String, String)>, String>,
{
    if settings.check_for_updates_automatically != Some(true) {
        return Vec::new();
    }

    let language = settings
        .language
        .as_deref()
        .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        .unwrap_or("en");

    if remote_services::should_run_automatic_update_check(Some(settings), now) {
        return vec![GuiShellAction::ApplyUpdateCheckResult(fetch_update_check(
            language,
        ))];
    }

    if settings.public_servers.as_ref().is_none_or(Vec::is_empty)
        && let Ok(servers) = fetch_public_servers(language)
        && !servers.is_empty()
    {
        return vec![GuiShellAction::ApplyStartupPublicServerCache(servers)];
    }

    Vec::new()
}

#[cfg(test)]
pub(super) fn startup_notice(settings: &StoredClientSettingsMvp) -> String {
    SyncplayGuiShellAppState::from_stored_settings(settings)
        .render_lines()
        .join("\n")
}

#[cfg(test)]
pub(super) fn shell_widget_preview(settings: &StoredClientSettingsMvp) -> String {
    let state = SyncplayGuiShellAppState::from_stored_settings(settings);
    let mut renderer = GuiWidgetTextPreviewRenderer::default();
    state.render_shell_widgets(&mut renderer);
    renderer.finish()
}

#[cfg(test)]
pub(super) fn startup_preview(settings: &StoredClientSettingsMvp) -> String {
    let mut host = GuiTextPreviewHost;
    run_gui_host(settings, &mut host)
}

#[cfg(test)]
pub(super) fn run_gui_host<Host: GuiAppHost>(
    settings: &StoredClientSettingsMvp,
    host: &mut Host,
) -> Host::Output {
    run_gui_host_with_startup_actions(settings, Vec::new(), host)
}

pub(super) fn run_gui_host_with_startup_actions_and_gui_state<Host: GuiAppHost>(
    settings: &StoredClientSettingsMvp,
    persisted_ui_state: Option<&GuiPersistedUiState>,
    startup_actions: Vec<GuiShellAction>,
    host: &mut Host,
) -> Host::Output {
    let mut startup_settings = settings.clone();
    if let Some(persisted_ui_state) = persisted_ui_state {
        persisted_ui_state.merge_into_startup_settings(&mut startup_settings);
    }
    let mut state = SyncplayGuiShellAppState::from_stored_settings(&startup_settings);
    if let Some(persisted_ui_state) = persisted_ui_state {
        state.apply_persisted_ui_state(persisted_ui_state);
    }
    for action in startup_actions {
        state.apply(action);
    }
    host.render(state)
}

#[cfg(test)]
pub(super) fn run_gui_host_with_startup_actions<Host: GuiAppHost>(
    settings: &StoredClientSettingsMvp,
    startup_actions: Vec<GuiShellAction>,
    host: &mut Host,
) -> Host::Output {
    run_gui_host_with_startup_actions_and_gui_state(settings, None, startup_actions, host)
}
