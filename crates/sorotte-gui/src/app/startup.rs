use std::{
    env,
    path::{Path, PathBuf},
};

use sorotte_client_app::app_boundary::language::normalized_legacy_runtime_language_tag_legacy_compatible;
use sorotte_client_app::app_boundary::{
    persistence::load_sorotte_ini_stored_client_settings_mvp_from_path,
    state::{
        StoredClientSettingsMvp, TlsPolicy,
        stored_client_settings_runtime_snapshot_legacy_compatible,
    },
    storage::{
        SorotteClientStoragePaths, SorotteClientStorageSource, current_sorotte_client_install_root,
        resolve_sorotte_client_storage_paths,
        resolve_sorotte_client_storage_paths_from_lookup_with_install_root,
    },
};

use super::GuiAppHost;
use super::native_host::GuiEframeNativeHost;
#[cfg(test)]
use super::native_host::GuiTextPreviewHost;
use super::remote_services;
use super::runtime_stack::GuiClientCoreChatSessionRuntimeAdapter;
use super::shell_state::{
    GuiConfigStorageRuntimeSnapshot, GuiShellAction, SorotteGuiShellAppState,
};
use super::startup_support::{
    GuiStartupConfigPathSource, GuiStartupPlayerIpcSource, GuiStartupPublicServerSource,
    env_trimmed, gui_client_core_chat_loopback_bootstrap_from_lookup,
    gui_client_core_chat_tcp_bootstrap_from_lookup,
};
use super::ui_state::{GuiPersistedUiState, load_gui_ui_state_from_root};
#[cfg(test)]
use super::widget_tree::GuiWidgetTextPreviewRenderer;

#[cfg(test)]
mod tests;

#[cfg(test)]
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
    gui_startup_settings_from_lookup_with_install_root(
        lookup,
        read_to_string,
        current_dir,
        || None,
        is_file,
        load_settings_at_path,
    )
}

fn gui_startup_settings_from_lookup_with_install_root<F, R, C, E, I, L>(
    lookup: F,
    read_to_string: R,
    current_dir: C,
    install_root: E,
    is_file: I,
    load_settings_at_path: L,
) -> Result<StoredClientSettingsMvp, String>
where
    F: Fn(&str) -> Option<String>,
    R: Fn(&str) -> Result<String, String>,
    C: Fn() -> Option<PathBuf>,
    E: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
    L: Fn(&Path) -> Result<Option<StoredClientSettingsMvp>, String>,
{
    let config_path_source =
        resolve_sorotte_gui_config_path_source_legacy_compatible_with_install_root(
            &lookup,
            current_dir,
            install_root,
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

#[cfg(test)]
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
            load_sorotte_ini_stored_client_settings_mvp_from_path(path)
                .map_err(|error| error.to_string())
        },
    )
}

fn gui_startup_settings_from_env() -> Result<StoredClientSettingsMvp, String> {
    gui_startup_settings_from_lookup_with_install_root(
        env_trimmed,
        |path| std::fs::read_to_string(path).map_err(|error| error.to_string()),
        || env::current_dir().ok(),
        current_sorotte_client_install_root,
        Path::is_file,
        |path| {
            load_sorotte_ini_stored_client_settings_mvp_from_path(path)
                .map_err(|error| error.to_string())
        },
    )
}

fn gui_startup_tcp_tls_policy(settings: &StoredClientSettingsMvp) -> TlsPolicy {
    stored_client_settings_runtime_snapshot_legacy_compatible(settings)
        .config
        .connection
        .tls_policy
}

pub(super) fn gui_startup_host_and_settings()
-> Result<(GuiEframeNativeHost, StoredClientSettingsMvp), String> {
    let config_path = resolve_sorotte_gui_config_path_legacy_compatible();
    let _ =
        remote_services::cleanup_update_staging_root(config_path.as_deref().and_then(Path::parent));
    let settings = gui_startup_settings_from_env()?;
    if let Some(bootstrap) = gui_client_core_chat_loopback_bootstrap_from_lookup(env_trimmed)? {
        let host = GuiEframeNativeHost::with_client_core_chat_loopback_session_for_config_path(
            bootstrap.username,
            bootstrap.room,
            config_path,
        )?;
        return Ok((host, settings));
    }
    let Some(bootstrap) = gui_client_core_chat_tcp_bootstrap_from_lookup(env_trimmed)? else {
        let host = GuiEframeNativeHost::with_queued_preview_runtime_for_config_path(config_path);
        return Ok((host, settings));
    };

    let host = GuiEframeNativeHost::with_client_core_chat_tcp_session_for_config_path(
        bootstrap.username.clone(),
        bootstrap.room.clone(),
        bootstrap.host_arg(),
        gui_startup_tcp_tls_policy(&settings),
        config_path,
    )?;
    Ok((host, settings))
}

pub(super) fn explicit_mpv_ipc_path_from_lookup<F>(lookup: &F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    GuiStartupPlayerIpcSource::from_lookup(lookup).map(|source| source.ipc_path().to_owned())
}

#[cfg(test)]
fn resolve_sorotte_gui_storage_paths_legacy_compatible_with<F, C, I>(
    lookup: &F,
    current_dir: C,
    is_file: I,
) -> Option<SorotteClientStoragePaths>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
{
    resolve_sorotte_client_storage_paths_from_lookup_with_install_root(
        lookup,
        current_dir,
        || None,
        is_file,
        |path| std::fs::read_to_string(path).ok(),
        None,
        None,
    )
}

fn resolve_sorotte_gui_storage_paths_legacy_compatible_with_install_root<F, C, E, I>(
    lookup: &F,
    current_dir: C,
    install_root: E,
    is_file: I,
) -> Option<SorotteClientStoragePaths>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    E: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
{
    resolve_sorotte_client_storage_paths_from_lookup_with_install_root(
        lookup,
        current_dir,
        install_root,
        is_file,
        |path| std::fs::read_to_string(path).ok(),
        None,
        None,
    )
}

fn resolve_sorotte_gui_storage_paths_legacy_compatible() -> Option<SorotteClientStoragePaths> {
    resolve_sorotte_client_storage_paths(None, None)
}

fn startup_config_path_source_from_storage_paths(
    paths: SorotteClientStoragePaths,
) -> GuiStartupConfigPathSource {
    match paths.source {
        SorotteClientStorageSource::CliConfigPath | SorotteClientStorageSource::EnvConfigPath => {
            GuiStartupConfigPathSource::Override(paths.config_path)
        }
        SorotteClientStorageSource::CliConfigRoot | SorotteClientStorageSource::EnvConfigRoot => {
            GuiStartupConfigPathSource::ConfigRootOverride(paths.config_path)
        }
        SorotteClientStorageSource::InstallConfigRoot => {
            GuiStartupConfigPathSource::InstallConfigRoot(paths.config_path)
        }
        SorotteClientStorageSource::PersistedConfigRoot => {
            GuiStartupConfigPathSource::PersistedConfigRoot(paths.config_path)
        }
        SorotteClientStorageSource::ConfigRootExisting => {
            GuiStartupConfigPathSource::ConfigRootExisting(paths.config_path)
        }
        SorotteClientStorageSource::DefaultConfigTarget => {
            GuiStartupConfigPathSource::DefaultConfigTarget(paths.config_path)
        }
    }
}

#[cfg(test)]
pub(super) fn resolve_sorotte_gui_config_path_source_legacy_compatible_with<F, C, I>(
    lookup: &F,
    current_dir: C,
    is_file: I,
) -> Option<GuiStartupConfigPathSource>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
{
    resolve_sorotte_gui_storage_paths_legacy_compatible_with(lookup, current_dir, is_file)
        .map(startup_config_path_source_from_storage_paths)
}

fn resolve_sorotte_gui_config_path_source_legacy_compatible_with_install_root<F, C, E, I>(
    lookup: &F,
    current_dir: C,
    install_root: E,
    is_file: I,
) -> Option<GuiStartupConfigPathSource>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    E: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
{
    resolve_sorotte_gui_storage_paths_legacy_compatible_with_install_root(
        lookup,
        current_dir,
        install_root,
        is_file,
    )
    .map(startup_config_path_source_from_storage_paths)
}

pub(super) fn resolve_sorotte_gui_config_path_legacy_compatible() -> Option<PathBuf> {
    resolve_sorotte_gui_storage_paths_legacy_compatible()
        .map(startup_config_path_source_from_storage_paths)
        .map(|source| source.resolved_path().to_path_buf())
}

pub(super) fn sorotte_gui_qsettings_root_from_env() -> Option<PathBuf> {
    resolve_sorotte_gui_config_path_legacy_compatible()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

pub(super) fn load_gui_ui_state_from_env() -> Result<Option<GuiPersistedUiState>, String> {
    let Some(root) = sorotte_gui_qsettings_root_from_env() else {
        return Ok(None);
    };
    load_gui_ui_state_from_root(&root)
}

#[cfg(test)]
pub(super) fn gui_startup_actions_from_lookup<F>(
    lookup: F,
    settings: &StoredClientSettingsMvp,
) -> Vec<GuiShellAction>
where
    F: Fn(&str) -> Option<String>,
{
    let storage_paths = resolve_sorotte_gui_storage_paths_legacy_compatible_with(
        &lookup,
        || env::current_dir().ok(),
        Path::is_file,
    );
    let config_path_source = storage_paths
        .clone()
        .map(startup_config_path_source_from_storage_paths);
    let mut messages = gui_startup_messages_from_lookup_and_config_path_source(
        &lookup,
        settings,
        config_path_source,
    );
    if GuiStartupPlayerIpcSource::from_lookup(&lookup).is_none() {
        messages.push(GuiStartupPlayerIpcSource::missing_startup_message());
    }
    storage_paths
        .as_ref()
        .map(|paths| {
            GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(
                GuiConfigStorageRuntimeSnapshot::from_storage_paths(paths),
            )
        })
        .into_iter()
        .chain(gui_startup_actions_from_messages(messages))
        .collect()
}

pub(super) fn gui_startup_actions_from_env(
    settings: &StoredClientSettingsMvp,
) -> Vec<GuiShellAction> {
    let storage_paths = resolve_sorotte_gui_storage_paths_legacy_compatible();
    let config_path_source = storage_paths
        .clone()
        .map(startup_config_path_source_from_storage_paths);
    let mut messages = gui_startup_messages_from_lookup_and_config_path_source(
        &env_trimmed,
        settings,
        config_path_source,
    );
    if GuiStartupPlayerIpcSource::from_lookup(&env_trimmed).is_none() {
        messages.push(GuiStartupPlayerIpcSource::missing_startup_message());
    }
    storage_paths
        .as_ref()
        .map(|paths| {
            GuiShellAction::ApplyGuiConfigStorageRuntimeSnapshot(
                GuiConfigStorageRuntimeSnapshot::from_storage_paths(paths),
            )
        })
        .into_iter()
        .chain(gui_startup_actions_from_messages(messages))
        .collect()
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
    messages
        .into_iter()
        .map(GuiShellAction::AnnounceSystemChatEvent)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StartupPublicServerOutcome {
    Loaded(Vec<(String, String)>),
    AlreadyCached,
    Failed(String),
}

pub(super) fn should_hydrate_startup_public_servers(settings: &StoredClientSettingsMvp) -> bool {
    // `None` means the cache has never been initialized. `Some([])` records an
    // explicit empty choice and must survive startup without a remote refill.
    settings.public_servers.is_none()
}

pub(super) fn gui_startup_public_server_outcome_with_fetcher<FPublicServers>(
    settings: &StoredClientSettingsMvp,
    fetch_public_servers: FPublicServers,
) -> StartupPublicServerOutcome
where
    FPublicServers: Fn(&str) -> Result<Vec<(String, String)>, String>,
{
    if !should_hydrate_startup_public_servers(settings) {
        return StartupPublicServerOutcome::AlreadyCached;
    }

    let language = settings
        .language
        .as_deref()
        .and_then(normalized_legacy_runtime_language_tag_legacy_compatible)
        .unwrap_or("en");

    match fetch_public_servers(language) {
        Ok(servers) if !servers.is_empty() => StartupPublicServerOutcome::Loaded(servers),
        Ok(_) => StartupPublicServerOutcome::Failed(
            "The public server service returned an empty list.".to_owned(),
        ),
        Err(error) => StartupPublicServerOutcome::Failed(error),
    }
}

#[cfg(test)]
pub(super) fn startup_notice(settings: &StoredClientSettingsMvp) -> String {
    SorotteGuiShellAppState::from_stored_settings(settings)
        .render_lines()
        .join("\n")
}

#[cfg(test)]
pub(super) fn shell_widget_preview(settings: &StoredClientSettingsMvp) -> String {
    let state = SorotteGuiShellAppState::from_stored_settings(settings);
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
    let mut state = SorotteGuiShellAppState::from_stored_settings(&startup_settings);
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
