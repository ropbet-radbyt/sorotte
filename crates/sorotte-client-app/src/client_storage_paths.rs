use std::path::{Path, PathBuf};

use anyhow::anyhow;

pub const SOROTTE_CONFIG_FILE_NAME: &str = "sorotte.ini";
pub const SOROTTE_CLIENT_CONFIG_PATH_ENV: &str = "SOROTTE_CLIENT_CONFIG_PATH";
pub const SOROTTE_CLIENT_CONFIG_ROOT_ENV: &str = "SOROTTE_CLIENT_CONFIG_ROOT";
pub const SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME: &str = "config-root.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorotteClientStorageSource {
    CliConfigPath,
    CliConfigRoot,
    EnvConfigPath,
    EnvConfigRoot,
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

pub fn sorotte_client_config_root_pointer_path(default_storage_root: &Path) -> PathBuf {
    default_storage_root.join(SOROTTE_CLIENT_CONFIG_ROOT_POINTER_FILE_NAME)
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
    let current_dir = current_dir();
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
    resolve_sorotte_client_storage_paths_from_lookup(
        &|name| std::env::var(name).ok(),
        || std::env::current_dir().ok(),
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
}
