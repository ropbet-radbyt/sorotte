//! Ownership shared by the GUI and its updater binary. A live lease protects a stage
//! regardless of its age. A short durable reservation covers process startup during handoff.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LEASE: &str = ".owner.lock";
const HANDOFF: &str = ".handoff.json";

#[derive(Debug)]
struct LeaseInner {
    path: PathBuf,
    _file: fs::File,
}

#[derive(Debug, Clone)]
pub struct StageLease(Arc<LeaseInner>);

impl PartialEq for StageLease {
    fn eq(&self, other: &Self) -> bool {
        self.0.path == other.0.path
    }
}
impl Eq for StageLease {}

#[derive(serde::Serialize, serde::Deserialize)]
struct Handoff {
    nonce: String,
    expires: u64,
}

fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|error| error.to_string())
}

fn regular(path: &Path, directory: bool) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(format!(
                "Update stage path is a reparse point: {}",
                path.display()
            ));
        }
    }
    if metadata.file_type().is_symlink()
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(format!(
            "Update stage path is not a regular {}: {}",
            if directory { "directory" } else { "file" },
            path.display()
        ));
    }
    Ok(())
}

/// Serialize stage creation, lease acquisition and removal, including the initial mkdir/open gap.
pub fn catalog_lock(updates_root: &Path) -> Result<fs::File, String> {
    let parent = updates_root
        .parent()
        .ok_or("Update staging root has no parent")?;
    regular(parent, true)?;
    let path = parent.join(".update-stages.lock");
    if path.try_exists().map_err(|error| error.to_string())? {
        regular(&path, false)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.lock().map_err(|error| error.to_string())?;
    Ok(file)
}

impl StageLease {
    /// The caller holds the catalog lock while creating the private stage directory.
    pub fn create(stage: &Path) -> Result<Self, String> {
        regular(stage, true)?;
        let file = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(stage.join(LEASE))
            .map_err(|error| error.to_string())?;
        file.lock_shared().map_err(|error| error.to_string())?;
        Ok(Self(Arc::new(LeaseInner {
            path: stage.to_path_buf(),
            _file: file,
        })))
    }

    pub fn acquire(stage: &Path) -> Result<Self, String> {
        let root = stage.parent().ok_or("Update stage has no parent")?;
        let _catalog = catalog_lock(root)?;
        regular(root, true)?;
        regular(stage, true)?;
        regular(&stage.join(LEASE), false)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(stage.join(LEASE))
            .map_err(|error| error.to_string())?;
        file.try_lock_shared().map_err(|error| error.to_string())?;
        Ok(Self(Arc::new(LeaseInner {
            path: stage.to_path_buf(),
            _file: file,
        })))
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn begin_handoff(&self) -> Result<String, String> {
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos()
        );
        let handoff = Handoff {
            nonce: nonce.clone(),
            expires: now()?.saturating_add(120),
        };
        let mut file =
            fs::File::create(self.path().join(HANDOFF)).map_err(|error| error.to_string())?;
        file.write_all(&serde_json::to_vec(&handoff).map_err(|error| error.to_string())?)
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())?;
        Ok(nonce)
    }

    fn ready_path(&self, nonce: &str) -> Result<PathBuf, String> {
        if nonce.is_empty()
            || nonce.len() > 80
            || !nonce
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'-')
        {
            return Err("Invalid update stage handoff identity".to_owned());
        }
        Ok(self.path().join(format!(".ready-{nonce}")))
    }

    /// Only the detached updater acknowledges, after acquiring its own live lease.
    pub fn acknowledge(&self, nonce: &str) -> Result<(), String> {
        let ready = self.ready_path(nonce)?;
        let handoff: Handoff = serde_json::from_slice(
            &fs::read(self.path().join(HANDOFF)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        if handoff.nonce != nonce {
            return Err("Update stage handoff identity changed".to_owned());
        }
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(ready)
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::remove_file(self.path().join(HANDOFF)).map_err(|error| error.to_string())
    }

    /// Called on an update worker, while its lease remains held. Parent exit is permitted
    /// only after the detached updater has acknowledged ownership.
    pub fn wait_for_handoff(
        &self,
        nonce: &str,
        child: &mut std::process::Child,
    ) -> Result<(), String> {
        let ready = self.ready_path(nonce)?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if ready.try_exists().map_err(|error| error.to_string())? {
                regular(&ready, false)?;
                return Ok(());
            }
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some_and(|status| !status.success())
            {
                return Err("Updater exited before accepting the staged update".to_owned());
            }
            if Instant::now() >= deadline {
                return Err("Updater did not accept the staged update within 30 seconds".to_owned());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// The caller holds the catalog lock through the subsequent removal. Live owners
/// always win; only an unlocked stage with no outstanding startup reservation is abandoned.
pub fn stage_is_live(stage: &Path) -> Result<bool, String> {
    regular(stage, true)?;
    let lease = stage.join(LEASE);
    if lease.try_exists().map_err(|error| error.to_string())? {
        regular(&lease, false)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lease)
            .map_err(|error| error.to_string())?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(true),
            Err(error) => return Err(error.to_string()),
        }
    }
    let reservation = stage.join(HANDOFF);
    if reservation
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        regular(&reservation, false)?;
        if fs::metadata(&reservation)
            .map_err(|error| error.to_string())?
            .len()
            > 512
        {
            return Err("Update handoff reservation is too large".to_owned());
        }
        let handoff: Handoff =
            serde_json::from_slice(&fs::read(reservation).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        return Ok(handoff.expires >= now()?);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_lease_outlasts_handoff_expiry_and_all_gui_clones() {
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        fs::create_dir(&stage).unwrap();
        let lease = StageLease::create(&stage).unwrap();
        let clone = lease.clone();
        lease.begin_handoff().unwrap();
        fs::write(stage.join(HANDOFF), br#"{"nonce":"1","expires":0}"#).unwrap();
        assert!(stage_is_live(&stage).unwrap());
        drop(lease);
        assert!(stage_is_live(&stage).unwrap());
        drop(clone);
        assert!(!stage_is_live(&stage).unwrap());
    }

    #[test]
    fn stage_lease_process_fixture() {
        let image = std::env::current_exe().unwrap();
        if image.file_stem().and_then(|name| name.to_str()) != Some("stage-owner-fixture")
            || !std::env::args().any(|arg| arg == "--exact")
            || !std::env::args()
                .any(|arg| arg == "update_stage::tests::stage_lease_process_fixture")
        {
            return;
        }
        let stage =
            PathBuf::from(std::env::var_os("SOROTTE_STAGE_FIXTURE_ROOT").expect("fixture stage"));
        let nonce = std::env::var("SOROTTE_STAGE_FIXTURE_HANDOFF").expect("fixture handoff");
        let lease = StageLease::acquire(&stage).unwrap();
        lease.acknowledge(&nonce).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !stage.join("release-owner").exists() {
            assert!(
                Instant::now() < deadline,
                "parent failed to release the controlled stage owner"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
