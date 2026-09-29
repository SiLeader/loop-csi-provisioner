use crate::controller::error::ControllerError;
use crate::lock::KeyedLocks;
use crate::mount::MountManager;
use crate::volume_id::{METADATA_DIR, VOLUME_DIR, parse_volume_id};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub(crate) struct ControllerOperator {
    default_size: i64,
    base_directory: String,
    mounter: Arc<MountManager>,
    /// Serializes operations per volume ID. It does not coordinate several controller
    /// processes sharing one backing directory; run a single active controller.
    locks: KeyedLocks,
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
    pub fn default_size(&self) -> i64 {
        self.default_size
    }
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

    pub async fn create_volume(
        &self,
        volume_id: &str,
        capacity: Option<i64>,
        url: &str,
    ) -> Result<i64, ControllerError> {
        let (id_url, _) = parse_volume_id(volume_id).ok_or(ControllerError::VolumeIdParse)?;
        if id_url != url {
            return Err(ControllerError::VolumeIdParse);
        }
        let _guard = self.locks.lock(volume_id).await;

        let size = capacity
            .filter(|size| *size > 0)
            .unwrap_or(self.default_size);
        let path = self.volume_file_path(volume_id).await?;
        if tokio::fs::try_exists(&path).await? {
            let existing = tokio::fs::metadata(&path).await?.len() as i64;
            if existing < size {
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
        let _guard = self.locks.lock(volume_id).await;

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
        let _guard = self.locks.lock(volume_id).await;

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
        let _guard = self.locks.lock(volume_id).await;

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

    pub async fn expand_volume(
        &self,
        volume_id: &str,
        size: Option<i64>,
    ) -> Result<i64, ControllerError> {
        let _guard = self.locks.lock(volume_id).await;

        let path = self.volume_file_path(volume_id).await?;
        if !tokio::fs::try_exists(&path).await? {
            return Err(ControllerError::NotFound(volume_id.to_string()));
        }
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .await?;
        let current = file.metadata().await?.len() as i64;
        let size = size.filter(|size| *size > 0).unwrap_or(current);
        if size < current {
            return Err(ControllerError::ExistingSize(current, size));
        }
        file.set_len(size as u64).await?;
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
            operator.create_volume(&id, Some(2048), url).await.unwrap(),
            2048
        );
        assert_eq!(
            operator.create_volume(&id, Some(1024), url).await.unwrap(),
            2048
        );
        assert_eq!(operator.volume_size(&id).await.unwrap(), 2048);
        assert!(matches!(
            operator.create_volume(&id, Some(4096), url).await,
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
        assert_eq!(operator.expand_volume(&id, Some(4096)).await.unwrap(), 4096);
        operator.delete_volume(&id).await.unwrap();
        operator.delete_volume(&id).await.unwrap();
        assert!(!storage.join(VOLUME_DIR).join("pvc-123.img").exists());
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
                .create_volume(&id, Some(2048), &f.url)
                .await
                .unwrap(),
            2048
        );
        assert!(!leftover.exists());
        assert_eq!(f.operator.volume_size(&id).await.unwrap(), 2048);
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_publish_attaches_to_one_node_only() {
        let f = Fixture::new("concurrent").await;
        let id = f.id("pvc-1");
        f.operator
            .create_volume(&id, Some(1024), &f.url)
            .await
            .unwrap();
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
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        assert_eq!(names, ["pvc-1.json"]);
        tokio::fs::remove_dir_all(&f.root).await.unwrap();
    }
}
