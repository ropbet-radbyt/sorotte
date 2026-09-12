use std::{env, path::PathBuf};

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
