use std::path::{Path, PathBuf};

use anyhow::anyhow;

pub const SOROTTE_CONFIG_FILE_NAME: &str = "sorotte.ini";
pub const SOROTTE_CLIENT_CONFIG_PATH_ENV: &str = "SOROTTE_CLIENT_CONFIG_PATH";
pub const SOROTTE_CLIENT_CONFIG_ROOT_ENV: &str = "SOROTTE_CLIENT_CONFIG_ROOT";
pub const SOROTTE_CLIENT_INSTALL_ROOT_ENV: &str = "SOROTTE_CLIENT_INSTALL_ROOT";
pub const SOROTTE_INSTALL_CONFIG_LOCATOR_FILE_NAME: &str = "syncplay.ini";
pub const SOROTTE_INSTALL_CONFIG_ROOT_KEY: &str = "configRoot";
pub const SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME: &str = "config-root.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorotteClientStorageSource {
    CliConfigPath,
    CliConfigRoot,
    EnvConfigPath,
    EnvConfigRoot,
    InstallConfigRoot,
    PersistedConfigRoot,
    ConfigRootExisting,
    DefaultConfigTarget,
}

impl SorotteClientStorageSource {
    pub fn is_external_override(self) -> bool {
        matches!(
            self,
            Self::CliConfigPath | Self::CliConfigRoot | Self::EnvConfigPath | Self::EnvConfigRoot
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::CliConfigPath => "CLI config path",
            Self::CliConfigRoot => "CLI config root",
            Self::EnvConfigPath => SOROTTE_CLIENT_CONFIG_PATH_ENV,
            Self::EnvConfigRoot => SOROTTE_CLIENT_CONFIG_ROOT_ENV,
            Self::InstallConfigRoot => "install syncplay.ini",
            Self::PersistedConfigRoot => "custom config root",
            Self::ConfigRootExisting => "default config root",
            Self::DefaultConfigTarget => "default config target",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SorotteClientStoragePaths {
    pub config_path: PathBuf,
    pub storage_root: PathBuf,
    pub default_storage_root: PathBuf,
    pub source: SorotteClientStorageSource,
}

impl SorotteClientStoragePaths {
    pub fn from_config_path(
        path: PathBuf,
        default_storage_root: PathBuf,
        source: SorotteClientStorageSource,
        current_dir: Option<PathBuf>,
    ) -> Self {
        let config_path = normalize_path(path, current_dir.clone());
        let storage_root = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or(current_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            config_path,
            storage_root,
            default_storage_root,
            source,
        }
    }

    pub fn from_root(
        root: PathBuf,
        default_storage_root: PathBuf,
        source: SorotteClientStorageSource,
        current_dir: Option<PathBuf>,
    ) -> Self {
        let storage_root = normalize_path(root, current_dir);
        let config_path = storage_root.join(SOROTTE_CONFIG_FILE_NAME);
        Self {
            config_path,
            storage_root,
            default_storage_root,
            source,
        }
    }
}

pub fn trim_non_empty(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub fn env_trimmed_from_lookup<F>(lookup: &F, name: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(name).and_then(trim_non_empty)
}

pub fn normalize_path(path: PathBuf, current_dir: Option<PathBuf>) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    current_dir.map_or(path.clone(), |dir| dir.join(path))
}

fn normalize_for_compare(path: &Path) -> String {
    let normalized = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

pub fn paths_equivalent(left: &Path, right: &Path) -> bool {
    normalize_for_compare(left) == normalize_for_compare(right)
}

fn fallback_storage_root_for_path(path: &Path, current_dir: Option<PathBuf>) -> PathBuf {
    normalize_path(path.to_path_buf(), current_dir.clone())
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or(current_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_sorotte_client_config_root_from_lookup<F>(lookup: &F) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    if cfg!(windows) {
        return env_trimmed_from_lookup(lookup, "APPDATA")
            .map(|root| PathBuf::from(root).join("Sorotte"));
    }
    if cfg!(target_os = "macos") {
        return env_trimmed_from_lookup(lookup, "HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("Sorotte")
        });
    }
    if let Some(xdg_config_home) = env_trimmed_from_lookup(lookup, "XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg_config_home).join("sorotte"));
    }
    env_trimmed_from_lookup(lookup, "HOME")
        .map(|home| PathBuf::from(home).join(".config").join("sorotte"))
}

pub fn default_sorotte_client_config_root() -> Option<PathBuf> {
    default_sorotte_client_config_root_from_lookup(&|name| std::env::var(name).ok())
}

pub fn sorotte_client_install_root_from_lookup<F>(
    lookup: &F,
    current_dir: Option<PathBuf>,
) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<String>,
{
    env_trimmed_from_lookup(lookup, SOROTTE_CLIENT_INSTALL_ROOT_ENV)
        .map(PathBuf::from)
        .map(|path| normalize_path(path, current_dir))
}

fn current_exe_is_cargo_test_harness(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(parent_name) = parent.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !parent_name.eq_ignore_ascii_case("deps") {
        return false;
    }
    let Some(profile_name) = parent
        .parent()
        .and_then(|profile| profile.file_name())
        .and_then(|name| name.to_str())
    else {
        return false;
    };
    if !profile_name.eq_ignore_ascii_case("debug") && !profile_name.eq_ignore_ascii_case("release")
    {
        return false;
    }
    let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some((_crate_name, hash_suffix)) = stem.rsplit_once('-') else {
        return false;
    };
    hash_suffix.len() >= 8 && hash_suffix.chars().all(|ch| ch.is_ascii_hexdigit())
}

pub fn current_sorotte_client_install_root() -> Option<PathBuf> {
    if let Some(root) =
        sorotte_client_install_root_from_lookup(&|name| std::env::var(name).ok(), None)
    {
        return Some(root);
    }
    let current_exe = std::env::current_exe().ok()?;
    if current_exe_is_cargo_test_harness(&current_exe) {
        return None;
    }
    current_exe.parent().map(Path::to_path_buf)
}

pub fn sorotte_client_config_root_pointer_path(default_storage_root: &Path) -> PathBuf {
    default_storage_root.join(SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME)
}

pub fn sorotte_client_install_locator_path(install_root: &Path) -> PathBuf {
    install_root.join(SOROTTE_INSTALL_CONFIG_LOCATOR_FILE_NAME)
}

fn unquote_locator_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let mut chars = trimmed.chars();
        let first = chars.next();
        let last = trimmed.chars().last();
        if matches!(
            (first, last),
            (Some('"'), Some('"')) | (Some('\''), Some('\''))
        ) {
            return trimmed[1..trimmed.len() - 1].trim().to_owned();
        }
    }
    trimmed.to_owned()
}

pub fn parse_sorotte_client_install_locator_config_root(
    contents: &str,
    install_root: &Path,
) -> Option<PathBuf> {
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with(';')
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.eq_ignore_ascii_case(SOROTTE_INSTALL_CONFIG_ROOT_KEY)
            && !key.eq_ignore_ascii_case("config_root")
            && !key.eq_ignore_ascii_case("settingsRoot")
            && !key.eq_ignore_ascii_case("settings_root")
        {
            continue;
        }
        let Some(value) = trim_non_empty(unquote_locator_value(value)) else {
            continue;
        };
        return Some(normalize_path(
            PathBuf::from(value),
            Some(install_root.to_path_buf()),
        ));
    }
    None
}

pub fn sorotte_client_install_locator_contents(install_root: &Path, storage_root: &Path) -> String {
    let rendered_root = if paths_equivalent(storage_root, install_root) {
        ".".to_owned()
    } else {
        storage_root.to_string_lossy().into_owned()
    };
    format!("[settings]\n{SOROTTE_INSTALL_CONFIG_ROOT_KEY} = {rendered_root}\n")
}

pub fn persist_sorotte_client_install_locator(
    install_root: &Path,
    storage_root: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(install_root).map_err(|error| {
        anyhow!(
            "failed creating Sorotte install directory {}: {error}",
            install_root.display()
        )
    })?;
    let locator_path = sorotte_client_install_locator_path(install_root);
    std::fs::write(
        &locator_path,
        sorotte_client_install_locator_contents(install_root, storage_root),
    )
    .map_err(|error| {
        anyhow!(
            "failed writing install config locator {}: {error}",
            locator_path.display()
        )
    })
}

pub fn ensure_sorotte_client_install_locator(
    install_root: &Path,
    default_storage_root: &Path,
) -> anyhow::Result<bool> {
    let locator_path = sorotte_client_install_locator_path(install_root);
    if locator_path.exists() {
        return Ok(false);
    }
    persist_sorotte_client_install_locator(install_root, default_storage_root)?;
    Ok(true)
}

pub fn load_sorotte_client_config_root_pointer_from_path(
    pointer_path: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    if !pointer_path.exists() {
        return Ok(None);
    }
    if !pointer_path.is_file() {
        return Err(anyhow!(
            "config-root pointer is not a file: {}",
            pointer_path.display()
        ));
    }
    let contents = std::fs::read_to_string(pointer_path).map_err(|error| {
        anyhow!(
            "failed reading config-root pointer {}: {error}",
            pointer_path.display()
        )
    })?;
    Ok(trim_non_empty(contents.lines().next().unwrap_or_default()).map(PathBuf::from))
}

pub fn persist_sorotte_client_config_root_pointer(
    default_storage_root: &Path,
    storage_root: &Path,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(default_storage_root).map_err(|error| {
        anyhow!(
            "failed creating default Sorotte config directory {}: {error}",
            default_storage_root.display()
        )
    })?;
    let pointer_path = sorotte_client_config_root_pointer_path(default_storage_root);
    std::fs::write(&pointer_path, storage_root.to_string_lossy().as_bytes()).map_err(|error| {
        anyhow!(
            "failed writing config-root pointer {}: {error}",
            pointer_path.display()
        )
    })
}

pub fn clear_sorotte_client_config_root_pointer(
    default_storage_root: &Path,
) -> anyhow::Result<bool> {
    let pointer_path = sorotte_client_config_root_pointer_path(default_storage_root);
    if !pointer_path.exists() {
        return Ok(false);
    }
    if !pointer_path.is_file() {
        return Err(anyhow!(
            "config-root pointer is not a file and cannot be cleared: {}",
            pointer_path.display()
        ));
    }
    std::fs::remove_file(&pointer_path).map_err(|error| {
        anyhow!(
            "failed clearing config-root pointer {}: {error}",
            pointer_path.display()
        )
    })?;
    Ok(true)
}

pub fn ensure_sorotte_client_storage_root(root: &Path) -> anyhow::Result<()> {
    if root.exists() && !root.is_dir() {
        return Err(anyhow!(
            "Sorotte config root is not a directory: {}",
            root.display()
        ));
    }
    std::fs::create_dir_all(root).map_err(|error| {
        anyhow!(
            "failed creating Sorotte config root {}: {error}",
            root.display()
        )
    })
}

pub fn resolve_sorotte_client_storage_paths_from_lookup<F, C, I, R>(
    lookup: &F,
    current_dir: C,
    is_file: I,
    read_to_string: R,
    cli_config_path: Option<PathBuf>,
    cli_config_root: Option<PathBuf>,
) -> Option<SorotteClientStoragePaths>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
    R: Fn(&Path) -> Option<String>,
{
    resolve_sorotte_client_storage_paths_from_lookup_with_install_root(
        lookup,
        current_dir,
        || None,
        is_file,
        read_to_string,
        cli_config_path,
        cli_config_root,
    )
}

pub fn resolve_sorotte_client_storage_paths_from_lookup_with_install_root<F, C, E, I, R>(
    lookup: &F,
    current_dir: C,
    install_root: E,
    is_file: I,
    read_to_string: R,
    cli_config_path: Option<PathBuf>,
    cli_config_root: Option<PathBuf>,
) -> Option<SorotteClientStoragePaths>
where
    F: Fn(&str) -> Option<String>,
    C: Fn() -> Option<PathBuf>,
    E: Fn() -> Option<PathBuf>,
    I: Fn(&Path) -> bool,
    R: Fn(&Path) -> Option<String>,
{
    let current_dir = current_dir();
    let install_root = install_root().map(|root| normalize_path(root, current_dir.clone()));
    let default_storage_root = default_sorotte_client_config_root_from_lookup(lookup);

    if let Some(path) = cli_config_path {
        let fallback_default_storage_root = default_storage_root
            .clone()
            .unwrap_or_else(|| fallback_storage_root_for_path(&path, current_dir.clone()));
        return Some(SorotteClientStoragePaths::from_config_path(
            path,
            fallback_default_storage_root,
            SorotteClientStorageSource::CliConfigPath,
            current_dir,
        ));
    }
    if let Some(root) = cli_config_root {
        let fallback_default_storage_root = default_storage_root
            .clone()
            .unwrap_or_else(|| normalize_path(root.clone(), current_dir.clone()));
        return Some(SorotteClientStoragePaths::from_root(
            root,
            fallback_default_storage_root,
            SorotteClientStorageSource::CliConfigRoot,
            current_dir,
        ));
    }
    if let Some(path) =
        env_trimmed_from_lookup(lookup, SOROTTE_CLIENT_CONFIG_PATH_ENV).map(PathBuf::from)
    {
        let fallback_default_storage_root = default_storage_root
            .clone()
            .unwrap_or_else(|| fallback_storage_root_for_path(&path, current_dir.clone()));
        return Some(SorotteClientStoragePaths::from_config_path(
            path,
            fallback_default_storage_root,
            SorotteClientStorageSource::EnvConfigPath,
            current_dir,
        ));
    }
    if let Some(root) =
        env_trimmed_from_lookup(lookup, SOROTTE_CLIENT_CONFIG_ROOT_ENV).map(PathBuf::from)
    {
        let fallback_default_storage_root = default_storage_root
            .clone()
            .unwrap_or_else(|| normalize_path(root.clone(), current_dir.clone()));
        return Some(SorotteClientStoragePaths::from_root(
            root,
            fallback_default_storage_root,
            SorotteClientStorageSource::EnvConfigRoot,
            current_dir,
        ));
    }

    let default_storage_root = default_storage_root?;
    if let Some(install_root) = install_root {
        let locator_path = sorotte_client_install_locator_path(&install_root);
        if is_file(&locator_path)
            && let Some(root) = read_to_string(&locator_path).and_then(|contents| {
                parse_sorotte_client_install_locator_config_root(&contents, &install_root)
            })
        {
            return Some(SorotteClientStoragePaths::from_root(
                root,
                default_storage_root,
                SorotteClientStorageSource::InstallConfigRoot,
                None,
            ));
        }
    }

    let pointer_path = sorotte_client_config_root_pointer_path(&default_storage_root);
    if is_file(&pointer_path)
        && let Some(root) = read_to_string(&pointer_path)
            .and_then(|contents| trim_non_empty(contents.lines().next().unwrap_or_default()))
            .map(PathBuf::from)
    {
        return Some(SorotteClientStoragePaths::from_root(
            root,
            default_storage_root,
            SorotteClientStorageSource::PersistedConfigRoot,
            current_dir,
        ));
    }

    let candidate = default_storage_root.join(SOROTTE_CONFIG_FILE_NAME);
    let source = if is_file(&candidate) {
        SorotteClientStorageSource::ConfigRootExisting
    } else {
        SorotteClientStorageSource::DefaultConfigTarget
    };
    Some(SorotteClientStoragePaths {
        config_path: candidate,
        storage_root: default_storage_root.clone(),
        default_storage_root,
        source,
    })
}

pub fn resolve_sorotte_client_storage_paths(
    cli_config_path: Option<PathBuf>,
    cli_config_root: Option<PathBuf>,
) -> Option<SorotteClientStoragePaths> {
    let install_root = current_sorotte_client_install_root();
    if let (Some(install_root), Some(default_storage_root)) =
        (install_root.as_ref(), default_sorotte_client_config_root())
    {
        let _ = ensure_sorotte_client_install_locator(install_root, &default_storage_root);
    }
    resolve_sorotte_client_storage_paths_from_lookup_with_install_root(
        &|name| std::env::var(name).ok(),
        || std::env::current_dir().ok(),
        || install_root.clone(),
        Path::is_file,
        |path| std::fs::read_to_string(path).ok(),
        cli_config_path,
        cli_config_root,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup(values: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |name| values.get(name).map(|value| (*value).to_owned())
    }

    fn base_env() -> HashMap<&'static str, &'static str> {
        let mut env = HashMap::new();
        if cfg!(windows) {
            env.insert("APPDATA", "C:/Users/test/AppData/Roaming");
        } else {
            env.insert("HOME", "/home/test");
        }
        env
    }

    #[test]
    fn storage_paths_use_cli_path_before_all_other_sources() {
        let mut env = base_env();
        env.insert(SOROTTE_CLIENT_CONFIG_PATH_ENV, "/env/sorotte.ini");
        env.insert(SOROTTE_CLIENT_CONFIG_ROOT_ENV, "/env-root");
        let paths = resolve_sorotte_client_storage_paths_from_lookup(
            &lookup(env),
            || Some(PathBuf::from("/cwd")),
            |_| false,
            |_| Some("/persisted-root".to_owned()),
            Some(PathBuf::from("relative/custom.ini")),
            Some(PathBuf::from("ignored")),
        )
        .expect("storage paths should resolve");

        assert_eq!(paths.source, SorotteClientStorageSource::CliConfigPath);
        assert_eq!(paths.config_path, PathBuf::from("/cwd/relative/custom.ini"));
        assert_eq!(paths.storage_root, PathBuf::from("/cwd/relative"));
    }

    #[test]
    fn storage_paths_use_cli_root_before_env_sources() {
        let mut env = base_env();
        env.insert(SOROTTE_CLIENT_CONFIG_PATH_ENV, "/env/sorotte.ini");
        let paths = resolve_sorotte_client_storage_paths_from_lookup(
            &lookup(env),
            || Some(PathBuf::from("/cwd")),
            |_| false,
            |_| None,
            None,
            Some(PathBuf::from("portable")),
        )
        .expect("storage paths should resolve");

        assert_eq!(paths.source, SorotteClientStorageSource::CliConfigRoot);
        assert_eq!(paths.storage_root, PathBuf::from("/cwd/portable"));
        assert_eq!(
            paths.config_path,
            PathBuf::from("/cwd/portable/sorotte.ini")
        );
    }

    #[test]
    fn storage_paths_use_env_path_before_env_root() {
        let mut env = base_env();
        env.insert(SOROTTE_CLIENT_CONFIG_PATH_ENV, "/env/sorotte.ini");
        env.insert(SOROTTE_CLIENT_CONFIG_ROOT_ENV, "/env-root");
        let paths = resolve_sorotte_client_storage_paths_from_lookup(
            &lookup(env),
            || Some(PathBuf::from("/cwd")),
            |_| false,
            |_| None,
            None,
            None,
        )
        .expect("storage paths should resolve");

        assert_eq!(paths.source, SorotteClientStorageSource::EnvConfigPath);
        assert_eq!(paths.config_path, PathBuf::from("/env/sorotte.ini"));
        assert_eq!(paths.storage_root, PathBuf::from("/env"));
    }

    #[test]
    fn storage_paths_use_env_root_before_persisted_root() {
        let mut env = base_env();
        env.insert(SOROTTE_CLIENT_CONFIG_ROOT_ENV, "/env-root");
        let paths = resolve_sorotte_client_storage_paths_from_lookup(
            &lookup(env),
            || Some(PathBuf::from("/cwd")),
            |_| true,
            |_| Some("/persisted-root".to_owned()),
            None,
            None,
        )
        .expect("storage paths should resolve");

        assert_eq!(paths.source, SorotteClientStorageSource::EnvConfigRoot);
        assert_eq!(paths.config_path, PathBuf::from("/env-root/sorotte.ini"));
    }

    #[test]
    fn storage_paths_use_install_locator_before_persisted_root() {
        let env = base_env();
        let paths = resolve_sorotte_client_storage_paths_from_lookup_with_install_root(
            &lookup(env),
            || Some(PathBuf::from("/cwd")),
            || Some(PathBuf::from("/install")),
            |path| {
                path.file_name().is_some_and(|name| {
                    name == SOROTTE_INSTALL_CONFIG_LOCATOR_FILE_NAME
                        || name == SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME
                })
            },
            |path| {
                if path
                    .file_name()
                    .is_some_and(|name| name == SOROTTE_INSTALL_CONFIG_LOCATOR_FILE_NAME)
                {
                    Some("[settings]\nconfigRoot = .\n".to_owned())
                } else {
                    Some("/persisted-root".to_owned())
                }
            },
            None,
            None,
        )
        .expect("storage paths should resolve");

        assert_eq!(paths.source, SorotteClientStorageSource::InstallConfigRoot);
        assert_eq!(paths.storage_root, PathBuf::from("/install"));
        assert_eq!(paths.config_path, PathBuf::from("/install/sorotte.ini"));
    }

    #[test]
    fn storage_paths_use_persisted_root_before_default_root() {
        let env = base_env();
        let paths = resolve_sorotte_client_storage_paths_from_lookup(
            &lookup(env),
            || Some(PathBuf::from("/cwd")),
            |path| {
                path.file_name()
                    .is_some_and(|name| name == SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME)
            },
            |_| Some("portable-root".to_owned()),
            None,
            None,
        )
        .expect("storage paths should resolve");

        assert_eq!(
            paths.source,
            SorotteClientStorageSource::PersistedConfigRoot
        );
        assert_eq!(paths.storage_root, PathBuf::from("/cwd/portable-root"));
        assert_eq!(
            paths.config_path,
            PathBuf::from("/cwd/portable-root/sorotte.ini")
        );
    }

    #[test]
    fn ensure_storage_root_rejects_existing_file() {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic enough for test")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sorotte-storage-root-file-test-{unique_suffix}"));
        std::fs::write(&root, b"not a directory").expect("test file should be written");

        let error = ensure_sorotte_client_storage_root(&root)
            .expect_err("existing file should not be accepted as a config root");
        assert!(error.to_string().contains("not a directory"));

        let _ = std::fs::remove_file(root);
    }

    #[test]
    fn install_locator_defaults_to_appdata_and_preserves_existing_file() {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be monotonic enough for test")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sorotte-install-locator-test-{unique_suffix}"));
        let install_root = root.join("install");
        let default_root = root.join("appdata").join("Sorotte");

        assert!(
            ensure_sorotte_client_install_locator(&install_root, &default_root)
                .expect("locator should be created")
        );
        let locator_path = sorotte_client_install_locator_path(&install_root);
        let contents = std::fs::read_to_string(&locator_path).expect("locator should be readable");
        assert_eq!(
            parse_sorotte_client_install_locator_config_root(&contents, &install_root),
            Some(default_root.clone())
        );

        std::fs::write(&locator_path, "[settings]\nconfigRoot = existing\n")
            .expect("locator should be overwritten by test setup");
        assert!(
            !ensure_sorotte_client_install_locator(&install_root, &default_root)
                .expect("existing locator should be preserved")
        );
        let contents = std::fs::read_to_string(&locator_path).expect("locator should be readable");
        assert!(contents.contains("existing"));

        persist_sorotte_client_install_locator(&install_root, &install_root)
            .expect("install-root locator should be writable");
        let contents = std::fs::read_to_string(&locator_path).expect("locator should be readable");
        assert!(
            contents.contains("configRoot = ."),
            "install-root selection should use a portable relative locator"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn current_exe_test_harness_detection_skips_cargo_deps_binaries() {
        assert!(current_exe_is_cargo_test_harness(Path::new(
            "/repo/target/debug/deps/sorotte_cli-1234abcd.exe"
        )));
        assert!(current_exe_is_cargo_test_harness(Path::new(
            "/repo/target/release/deps/sorotte_gui-abcdef1234567890"
        )));
        assert!(!current_exe_is_cargo_test_harness(Path::new(
            "/repo/target/debug/sorotte-gui.exe"
        )));
        assert!(!current_exe_is_cargo_test_harness(Path::new(
            "/repo/bin/deps/sorotte-gui.exe"
        )));
    }
}
