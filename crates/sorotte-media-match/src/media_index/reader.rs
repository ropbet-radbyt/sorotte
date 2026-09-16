use super::*;

/// Reuses one validated connection for fingerprint and inventory reads. All methods may
/// perform disk I/O and must be called on a worker, never an interaction thread.
pub struct MediaIndexRecordReader {
    service: MediaIndexService,
    admitted: Option<AdmittedIndex>,
    #[cfg(test)]
    admissions: usize,
}

struct AdmittedIndex {
    manifest: Option<MediaIndexManifest>,
    file_version: IndexFileVersion,
    data_version: i64,
    session: MediaIndexSession,
}

#[derive(PartialEq, Eq)]
struct IndexFileVersion {
    bytes: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}

fn index_file_version(root: &Path) -> Result<IndexFileVersion, String> {
    let path = media_match_v3_index_path(root);
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata_is_reparse_or_symlink(&metadata) {
        return Err(format!(
            "refusing non-file or redirected media index '{}'",
            path.display()
        ));
    }
    Ok(IndexFileVersion {
        bytes: metadata.len(),
        modified: metadata.modified().map_err(|error| error.to_string())?,
        created: metadata.created().ok(),
    })
}

fn sqlite_data_version(session: &MediaIndexSession) -> Result<i64, String> {
    session
        .connection
        .pragma_query_value(None, "data_version", |row| row.get(0))
        .map_err(|error| error.to_string())
}

impl MediaIndexRecordReader {
    #[cfg(test)]
    pub(crate) fn admission_count(&self) -> usize {
        self.admissions
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            service: MediaIndexService::new(root),
            admitted: None,
            #[cfg(test)]
            admissions: 0,
        }
    }

    pub fn load_record(
        &mut self,
        normalized_path: &str,
        extraction_settings: &MediaExtractionSettings,
        modified_unix_millis: u64,
        size_bytes: u64,
    ) -> Result<Option<MediaFingerprintRecord>, String> {
        self.with_session(|session| {
            session.load_record(
                normalized_path,
                extraction_settings,
                modified_unix_millis,
                size_bytes,
            )
        })
    }

    pub fn inventory_paths(&mut self) -> Result<Vec<String>, String> {
        self.with_session(MediaIndexSession::inventory_paths)
    }

    fn with_session<T>(
        &mut self,
        read: impl FnOnce(&MediaIndexSession) -> Result<T, String>,
    ) -> Result<T, String> {
        let result = self.with_session_inner(read);
        if result.is_err() {
            // Errors re-enter normal admission/recovery on the next request.
            self.admitted = None;
        }
        result
    }

    fn with_session_inner<T>(
        &mut self,
        read: impl FnOnce(&MediaIndexSession) -> Result<T, String>,
    ) -> Result<T, String> {
        let _lock = acquire_media_index_activation_lock(self.service.root())?;
        let manifest = read_best_media_index_manifest(self.service.root())?;
        let unchanged = self.admitted.as_ref().is_some_and(|index| {
            index.manifest == manifest
                && manifest.as_ref().is_none_or(|manifest| {
                    !media_index_manifest_slots_need_repair(self.service.root(), manifest)
                        && existing_media_index_generations_root(self.service.root())
                            .is_ok_and(|root| root.is_some())
                })
                && validate_real_media_index_directory(&index.session.root).is_ok()
                && index_file_version(&index.session.root)
                    .is_ok_and(|version| version == index.file_version)
                && sqlite_data_version(&index.session)
                    .is_ok_and(|version| version == index.data_version)
        });
        if !unchanged {
            // Release the old generation before normal recovery/collection runs.
            self.admitted = None;
            let resolved = resolve_media_index_root_locked(self.service.root())?;
            let (root, connection) = match resolved.root {
                ResolvedMediaIndexRoot::ExistingGeneration(root) => {
                    let connection = open_existing_media_match_v3_index(&root)?;
                    (root, connection)
                }
                ResolvedMediaIndexRoot::DirectRoot(root) => {
                    let connection = open_media_match_v3_index(&root)?;
                    (root, connection)
                }
            };
            let session = MediaIndexSession { root, connection };
            self.admitted = Some(AdmittedIndex {
                manifest: read_best_media_index_manifest(self.service.root())?,
                file_version: index_file_version(&session.root)?,
                data_version: sqlite_data_version(&session)?,
                session,
            });
            #[cfg(test)]
            {
                self.admissions += 1;
            }
        }
        read(
            &self
                .admitted
                .as_ref()
                .expect("index admitted above")
                .session,
        )
    }
}
