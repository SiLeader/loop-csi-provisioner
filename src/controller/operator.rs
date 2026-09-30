use crate::controller::error::ControllerError;
use crate::lock::{FileLock, KeyedLocks};
use crate::mount::MountManager;
use crate::volume_id::{METADATA_DIR, VOLUME_DIR, parse_volume_id};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::OwnedMutexGuard;

pub(crate) struct ControllerOperator {
    default_size: i64,
    base_directory: String,
    mounter: Arc<MountManager>,
    /// Serializes operations per volume ID inside this process; [`Self::lock_volume`] adds a
    /// file lock on the backing storage for other processes.
    locks: KeyedLocks,
}

/// Volume sizes are rounded up to whole ext4 blocks, so the loop device and the filesystem
/// cover the entire image and the reported capacity is what the volume can actually hold.
const BLOCK_SIZE: u64 = 4096;

/// Volumes share a fixed set of lock files in the metadata directory. Deleting a volume's own
/// lock file would be unsafe: a process still holding the unlinked file and one that created
/// a new file at the same path could both believe they hold the lock.
const LOCK_FILES: u64 = 64;

/// How long to wait for another controller process to finish with a volume before
/// returning ABORTED, which the CO retries.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Holds a volume exclusively against both this process and other controller processes.
struct VolumeGuard {
    _file: FileLock,
    _local: OwnedMutexGuard<()>,
}

/// Returns the lock file index for a volume name. FNV-1a keeps it stable across builds and
/// hosts, which `DefaultHasher` does not promise.
fn lock_file_index(name: &str) -> u64 {
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    hash % LOCK_FILES
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    volume_id: String,
    attached_node: Option<String>,
}

/// Writes `contents` to a temporary sibling and renames it over `path`, so readers see
/// either the old or the new file and a crash never leaves a truncated `path`.
async fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let result = async {
        let mut file = tokio::fs::File::create(&tmp).await?;
        file.write_all(contents).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

impl ControllerOperator {
    pub async fn new(
        default_size: i64,
        base_directory: String,
        mounter: Arc<MountManager>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            default_size,
            base_directory,
            mounter,
            locks: KeyedLocks::default(),
        })
    }

    /// Returns the image size for a capacity range, rounded to whole blocks. With no required
    /// size, choose the default or the largest whole-block size below the limit.
    fn resolve_size(&self, required: i64, limit: i64) -> Result<i64, ControllerError> {
        let wanted = if required > 0 {
            required
        } else if limit > 0 {
            self.default_size.min(limit - limit % BLOCK_SIZE as i64)
        } else {
            self.default_size
        };
        let size = u64::try_from(wanted)
            .ok()
            .filter(|size| *size > 0)
            .and_then(|size| size.checked_next_multiple_of(BLOCK_SIZE))
            .and_then(|size| i64::try_from(size).ok())
            .ok_or(ControllerError::ExceedsLimit(wanted, i64::MAX))?;
        if limit > 0 && size > limit {
            return Err(ControllerError::ExceedsLimit(size, limit));
        }
        Ok(size)
    }

    async fn volume_file_path(&self, volume_id: &str) -> Result<PathBuf, ControllerError> {
        let Some((url, volume_id)) = parse_volume_id(volume_id) else {
            return Err(ControllerError::VolumeIdParse);
        };

        let mount_point = self.mounter.mount(url, &self.base_directory).await?;
        let storage_base_directory = PathBuf::from(&mount_point).join(VOLUME_DIR);
        Ok(storage_base_directory.join(format!("{}.img", volume_id)))
    }

    async fn metadata_file_path(&self, volume_id: &str) -> Result<PathBuf, ControllerError> {
        let Some((url, volume_id)) = parse_volume_id(volume_id) else {
            return Err(ControllerError::VolumeIdParse);
        };

        let mount_point = self.mounter.mount(url, &self.base_directory).await?;
        let metadata_base_directory = PathBuf::from(&mount_point).join(METADATA_DIR);
        Ok(metadata_base_directory.join(format!("{}.json", volume_id)))
    }

    async fn lock_volume(&self, volume_id: &str) -> Result<VolumeGuard, ControllerError> {
        let Some((url, name)) = parse_volume_id(volume_id) else {
            return Err(ControllerError::VolumeIdParse);
        };
        let local = self.locks.lock(volume_id).await;

        let mount_point = self.mounter.mount(url, &self.base_directory).await?;
        let path = PathBuf::from(&mount_point)
            .join(METADATA_DIR)
            .join(format!("lock-{:02x}.lock", lock_file_index(name)));
        let Some(file) = FileLock::acquire(path, LOCK_TIMEOUT).await? else {
            return Err(ControllerError::Busy(volume_id.to_string()));
        };
        Ok(VolumeGuard {
            _file: file,
            _local: local,
        })
    }

    async fn load_metadata(&self, volume_id: &str) -> Result<Option<Metadata>, ControllerError> {
        let metadata_file_path = self.metadata_file_path(volume_id).await?;
        match tokio::fs::read(&metadata_file_path).await {
            Ok(contents) => Ok(Some(serde_json::from_slice(&contents)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn save_metadata(&self, metadata: &Metadata) -> Result<(), ControllerError> {
        let metadata_file_path = self.metadata_file_path(&metadata.volume_id).await?;
        write_atomic(&metadata_file_path, &serde_json::to_vec(metadata)?).await?;
        Ok(())
    }

    /// Creates the image, or returns the size of an existing one that satisfies the range.
    /// `required` and `limit` follow CSI `CapacityRange`: 0 means unspecified.
    pub async fn create_volume(
        &self,
        volume_id: &str,
        required: i64,
        limit: i64,
        url: &str,
    ) -> Result<i64, ControllerError> {
        let (id_url, _) = parse_volume_id(volume_id).ok_or(ControllerError::VolumeIdParse)?;
        if id_url != url {
            return Err(ControllerError::VolumeIdParse);
        }
        let size = self.resolve_size(required, limit)?;
        let _guard = self.lock_volume(volume_id).await?;

        let path = self.volume_file_path(volume_id).await?;
        if tokio::fs::try_exists(&path).await? {
            let existing = tokio::fs::metadata(&path).await?.len() as i64;
            if existing < size || (limit > 0 && existing > limit) {
                return Err(ControllerError::ExistingSize(existing, size));
            }
            return Ok(existing);
        }

        // Size the file under a temporary name and rename it, so a crash in between never
        // leaves a zero-length image that later CreateVolume calls would reject forever.
        let mut tmp_name = path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let result = async {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp)
                .await?;
            file.set_len(size as u64).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&tmp, &path).await
        }
        .await;
        if let Err(e) = result {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e.into());
        }
        Ok(size)
    }

    pub async fn delete_volume(&self, volume_id: &str) -> Result<(), ControllerError> {
        let _guard = self.lock_volume(volume_id).await?;

        let metadata = self.load_metadata(volume_id).await?;
        if let Some(metadata) = metadata
            && let Some(attached_node) = metadata.attached_node
        {
            return Err(ControllerError::StillAttached {
                volume_id: volume_id.to_string(),
                attached_node,
            });
        }
        let path = self.volume_file_path(volume_id).await?;
        if tokio::fs::try_exists(&path).await? {
            tokio::fs::remove_file(path).await?;
        }

        let metadata_file_path = self.metadata_file_path(volume_id).await?;
        if tokio::fs::try_exists(&metadata_file_path).await? {
            tokio::fs::remove_file(metadata_file_path).await?;
        }

        Ok(())
    }

    pub async fn publish_volume(
        &self,
        volume_id: &str,
        node_id: &str,
    ) -> Result<(), ControllerError> {
        let _guard = self.lock_volume(volume_id).await?;

        if !tokio::fs::try_exists(self.volume_file_path(volume_id).await?).await? {
            return Err(ControllerError::NotFound(volume_id.to_string()));
        }
        if let Some(metadata) = self.load_metadata(volume_id).await?
            && let Some(attached_node) = &metadata.attached_node
        {
            if attached_node != node_id {
                return Err(ControllerError::AlreadyAttached {
                    volume_id: volume_id.to_string(),
                    attached_node: attached_node.clone(),
                    requested_node: node_id.to_string(),
                });
            }
            return Ok(());
        }
        self.save_metadata(&Metadata {
            volume_id: volume_id.to_string(),
            attached_node: Some(node_id.to_string()),
        })
        .await
    }

    pub async fn unpublish_volume(
        &self,
        volume_id: &str,
        node_id: &str,
    ) -> Result<(), ControllerError> {
        let _guard = self.lock_volume(volume_id).await?;

        if let Some(mut metadata) = self.load_metadata(volume_id).await?
            && (node_id.is_empty() || metadata.attached_node.as_deref() == Some(node_id))
        {
            metadata.attached_node = None;
            self.save_metadata(&metadata).await?;
        }
        Ok(())
    }

    pub async fn volume_size(&self, volume_id: &str) -> Result<i64, ControllerError> {
        let path = self.volume_file_path(volume_id).await?;
        match tokio::fs::metadata(path).await {
            Ok(metadata) => Ok(metadata.len() as i64),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(ControllerError::NotFound(volume_id.to_string()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Grows the image to `required` (rounded up to [`BLOCK_SIZE`]). Asking for less than
    /// the current size is not an error: the volume already satisfies the request.
    pub async fn expand_volume(
        &self,
        volume_id: &str,
        required: i64,
        limit: i64,
    ) -> Result<i64, ControllerError> {
        let _guard = self.lock_volume(volume_id).await?;

        let path = self.volume_file_path(volume_id).await?;
        if !tokio::fs::try_exists(&path).await? {
            return Err(ControllerError::NotFound(volume_id.to_string()));
        }
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .await?;
        let current = file.metadata().await?.len() as i64;
        if required <= current {
            if limit > 0 && current > limit {
                return Err(ControllerError::ExceedsLimit(current, limit));
            }
            return Ok(current);
        }
        let size = self.resolve_size(required, limit)?;
        file.set_len(size as u64).await?;
        file.sync_all().await?;
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        storage: PathBuf,
        url: String,
        operator: ControllerOperator,
    }

    impl Fixture {
        async fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "loop-csi-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let storage = root.join("storage");
            let mounts = root.join("mounts");
            tokio::fs::create_dir_all(storage.join(VOLUME_DIR))
                .await
                .unwrap();
            tokio::fs::create_dir_all(storage.join(METADATA_DIR))
                .await
                .unwrap();
            let url = format!("file://{}", storage.display());
            let operator = ControllerOperator::new(
                1024,
                mounts.to_string_lossy().to_string(),
                Arc::new(MountManager::default()),
            )
            .await
            .unwrap();
            Self {
                root,
                storage,
                url,
                operator,
            }
        }

        fn id(&self, name: &str) -> String {
            format!("{}:{name}", self.url)
        }
    }

    #[tokio::test]
    async fn local_volume_lifecycle() {
        let f = Fixture::new("lifecycle").await;
        let (operator, url, storage) = (&f.operator, &f.url, &f.storage);
        let id = f.id("pvc-123");

        assert_eq!(
            operator.create_volume(&id, 8192, 0, url).await.unwrap(),
            8192
        );
        assert_eq!(
            operator.create_volume(&id, 4096, 0, url).await.unwrap(),
            8192
        );
        assert_eq!(operator.volume_size(&id).await.unwrap(), 8192);
        assert!(matches!(
            operator.create_volume(&id, 16384, 0, url).await,
            Err(ControllerError::ExistingSize(..))
        ));
        // An existing volume larger than the limit does not satisfy the request either.
        assert!(matches!(
            operator.create_volume(&id, 4096, 4096, url).await,
            Err(ControllerError::ExistingSize(..))
        ));
        operator.publish_volume(&id, "node-a").await.unwrap();
        operator.publish_volume(&id, "node-a").await.unwrap();
        assert!(matches!(
            operator.publish_volume(&id, "node-b").await,
            Err(ControllerError::AlreadyAttached { .. })
        ));
        assert!(matches!(
            operator.delete_volume(&id).await,
            Err(ControllerError::StillAttached { .. })
        ));
        operator.unpublish_volume(&id, "node-a").await.unwrap();
        assert_eq!(operator.expand_volume(&id, 10000, 0).await.unwrap(), 12288);
        // Shrinking is a no-op that reports the current size.
        assert_eq!(operator.expand_volume(&id, 4096, 0).await.unwrap(), 12288);
        assert!(matches!(
            operator.expand_volume(&id, 4096, 8192).await,
            Err(ControllerError::ExceedsLimit(..))
        ));
        assert!(matches!(
            operator.expand_volume(&id, 20000, 20000).await,
            Err(ControllerError::ExceedsLimit(..))
        ));
        assert_eq!(operator.volume_size(&id).await.unwrap(), 12288);
        operator.delete_volume(&id).await.unwrap();
        operator.delete_volume(&id).await.unwrap();
        assert!(!storage.join(VOLUME_DIR).join("pvc-123.img").exists());
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }

    #[tokio::test]
    async fn sizes_are_rounded_to_blocks_within_the_limit() {
        let mut f = Fixture::new("sizes").await;
        let op = &f.operator;
        assert_eq!(op.resolve_size(0, 0).unwrap(), 4096); // default 1024, rounded
        assert_eq!(op.resolve_size(4096, 0).unwrap(), 4096);
        assert_eq!(op.resolve_size(4097, 0).unwrap(), 8192);
        assert_eq!(op.resolve_size(1, 4096).unwrap(), 4096);
        assert_eq!(op.resolve_size(0, 5000).unwrap(), 4096);
        // No whole block fits between required and limit.
        assert!(op.resolve_size(4097, 5000).is_err());
        assert!(op.resolve_size(0, 1000).is_err());
        assert!(op.resolve_size(i64::MAX, 0).is_err());
        assert!(matches!(
            op.create_volume(&f.id("pvc-1"), 4097, 5000, &f.url).await,
            Err(ControllerError::ExceedsLimit(..))
        ));
        f.operator.default_size = 16384;
        assert_eq!(f.operator.resolve_size(0, 5000).unwrap(), 4096);
        assert_eq!(
            f.operator
                .create_volume(&f.id("limited"), 0, 5000, &f.url)
                .await
                .unwrap(),
            4096
        );
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }

    #[tokio::test]
    async fn interrupted_create_does_not_block_retry() {
        let f = Fixture::new("interrupted").await;
        let id = f.id("pvc-1");
        // A crash between creating and sizing leaves only the temporary file behind.
        let leftover = f.storage.join(VOLUME_DIR).join("pvc-1.img.tmp");
        tokio::fs::write(&leftover, b"junk").await.unwrap();

        assert_eq!(
            f.operator
                .create_volume(&id, 8192, 0, &f.url)
                .await
                .unwrap(),
            8192
        );
        assert!(!leftover.exists());
        assert_eq!(f.operator.volume_size(&id).await.unwrap(), 8192);
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_publish_attaches_to_one_node_only() {
        let f = Fixture::new("concurrent").await;
        let id = f.id("pvc-1");
        f.operator.create_volume(&id, 0, 0, &f.url).await.unwrap();
        let operator = Arc::new(f.operator);

        let tasks: Vec<_> = (0..16)
            .map(|i| {
                let operator = operator.clone();
                let id = id.clone();
                tokio::spawn(
                    async move { operator.publish_volume(&id, &format!("node-{i}")).await },
                )
            })
            .collect();
        let mut succeeded = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => succeeded += 1,
                Err(ControllerError::AlreadyAttached { .. }) => {}
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert_eq!(succeeded, 1);
        // Metadata is replaced atomically and never left half-written.
        let dir = f.storage.join(METADATA_DIR);
        let mut names = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".lock") {
                names.push(name);
            }
        }
        assert_eq!(names, ["pvc-1.json"]);
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }

    #[tokio::test]
    async fn controllers_sharing_storage_attach_to_one_node_only() {
        let f = Fixture::new("shared").await;
        let id = f.id("pvc-1");
        f.operator.create_volume(&id, 0, 0, &f.url).await.unwrap();
        // A second controller process with its own mounts of the same storage.
        let other = ControllerOperator::new(
            1024,
            f.root.join("other-mounts").to_string_lossy().to_string(),
            Arc::new(MountManager::default()),
        )
        .await
        .unwrap();
        let operators = [Arc::new(f.operator), Arc::new(other)];

        let tasks: Vec<_> = (0..16)
            .map(|i| {
                let operator = operators[i % 2].clone();
                let id = id.clone();
                tokio::spawn(
                    async move { operator.publish_volume(&id, &format!("node-{i}")).await },
                )
            })
            .collect();
        let mut succeeded = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(()) => succeeded += 1,
                Err(ControllerError::AlreadyAttached { .. }) => {}
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert_eq!(succeeded, 1);
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }

    #[test]
    fn lock_file_index_is_stable() {
        // Controllers of different builds must agree on the lock file for a volume.
        assert_eq!(lock_file_index("pvc-1"), 42);
        assert!((0..100).all(|i| lock_file_index(&format!("pvc-{i}")) < LOCK_FILES));
    }
}
