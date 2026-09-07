use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use reqwest::blocking::Client;

use crate::app::child_process::configure_gui_child_process;

use super::{STREAM_HELPER_DOWNLOAD_TIMEOUT, STREAM_HELPER_USER_AGENT};

pub(in crate::app::stream_support) fn helper_http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(STREAM_HELPER_DOWNLOAD_TIMEOUT)
        .user_agent(STREAM_HELPER_USER_AGENT)
        .build()
        .map_err(|error| format!("failed to build stream-helper HTTP client: {error}"))
}

pub(in crate::app::stream_support) fn download_bytes(
    client: &Client,
    url: &str,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|error| format!("failed to download '{url}': {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "failed to download '{url}': HTTP {}",
            response.status()
        ));
    }
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|error| format!("failed reading '{url}' response body: {error}"))
}

pub(in crate::app::stream_support) fn download_to_path(
    client: &Client,
    url: &str,
    path: &Path,
) -> Result<(), String> {
    let bytes = download_bytes(client, url)?;
    fs::write(path, bytes).map_err(|error| {
        format!(
            "failed to write downloaded stream helper file '{}': {error}",
            path.display()
        )
    })
}

pub(in crate::app::stream_support) fn probe_executable_version(
    path: &Path,
    args: &[&str],
) -> Result<String, String> {
    let mut command = Command::new(path);
    configure_gui_child_process(&mut command);
    let output = command
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("failed to start '{}': {error}", path.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let detail = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("exit status {}", output.status)
        };
        return Err(detail);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    Ok(version)
}

#[cfg(test)]
type StreamHelperPathLookup = fn(&[&str]) -> Option<PathBuf>;

#[cfg(test)]
std::thread_local! {
    static TEST_PATH_LOOKUP: std::cell::Cell<Option<StreamHelperPathLookup>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(in crate::app) fn with_stream_helper_path_lookup_for_test<T>(
    lookup: StreamHelperPathLookup,
    action: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<StreamHelperPathLookup>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PATH_LOOKUP.with(|slot| slot.set(self.0));
        }
    }

    let _restore = Restore(TEST_PATH_LOOKUP.with(|slot| slot.replace(Some(lookup))));
    action()
}

pub(in crate::app::stream_support) fn find_executable_on_path(
    candidates: &[&str],
) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(lookup) = TEST_PATH_LOOKUP.with(std::cell::Cell::get) {
        return lookup(candidates);
    }
    let path_env = env::var_os("PATH")?;
    for directory in env::split_paths(&path_env) {
        for candidate in candidates {
            let path = directory.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod path_lookup_tests {
    use super::{find_executable_on_path, with_stream_helper_path_lookup_for_test};
    use std::path::PathBuf;

    #[test]
    fn explicit_path_lookup_is_thread_local_and_restored_after_nested_unwind() {
        assert_eq!(find_executable_on_path(&[]), None);
        with_stream_helper_path_lookup_for_test(
            |_| Some(PathBuf::from("outer-fixture-tool")),
            || {
                assert_eq!(
                    find_executable_on_path(&[]),
                    Some(PathBuf::from("outer-fixture-tool"))
                );
                let other_thread = std::thread::spawn(|| find_executable_on_path(&[]));
                assert_eq!(other_thread.join().unwrap(), None);
                let panic = std::panic::catch_unwind(|| {
                    with_stream_helper_path_lookup_for_test(
                        |_| Some(PathBuf::from("inner-fixture-tool")),
                        || {
                            assert_eq!(
                                find_executable_on_path(&[]),
                                Some(PathBuf::from("inner-fixture-tool"))
                            );
                            panic!("deliberate resolver-scope unwind");
                        },
                    );
                });
                let payload = panic.expect_err("the nested scope must reach its deliberate panic");
                assert_eq!(
                    payload.downcast_ref::<&str>().copied(),
                    Some("deliberate resolver-scope unwind")
                );
                assert_eq!(
                    find_executable_on_path(&[]),
                    Some(PathBuf::from("outer-fixture-tool"))
                );
            },
        );
        assert_eq!(find_executable_on_path(&[]), None);
    }
}
