use crate::controller::error::ControllerError;
use crate::mount::MountManager;
use crate::volume_id::{METADATA_DIR, VOLUME_DIR, parse_volume_id};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::PathBuf;

pub(crate) struct ControllerOperator {
    default_size: i64,
    base_directory: String,
    mounter: MountManager,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    volume_id: String,
    attached_node: Option<String>,
}

impl ControllerOperator {
    pub fn default_size(&self) -> i64 {
        self.default_size
    }
    pub async fn new(default_size: i64, base_directory: String) -> std::io::Result<Self> {
        Ok(Self {
            default_size,
            base_directory,
            mounter: MountManager::default(),
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
        if !metadata_file_path.exists() {
            return Ok(None);
        }
        let file = File::open(&metadata_file_path)?;
        let metadata: Metadata = serde_json::from_reader(file)?;
        Ok(Some(metadata))
    }

    async fn save_metadata(&self, metadata: &Metadata) -> Result<(), ControllerError> {
        let metadata_file_path = self.metadata_file_path(&metadata.volume_id).await?;
        let file = File::create(&metadata_file_path)?;
        serde_json::to_writer(file, metadata)?;
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
        let file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await?;
        file.set_len(size as u64).await?;
        Ok(size)
    }

    pub async fn delete_volume(&self, volume_id: &str) -> Result<(), ControllerError> {
        let metadata = self.load_metadata(volume_id).await?;
        if let Some(metadata) = metadata
            && metadata.attached_node.is_some()
        {
            return Err(ControllerError::StillAttached {
                volume_id: volume_id.to_string(),
                attached_node: metadata.attached_node.unwrap(),
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
        if !tokio::fs::try_exists(self.volume_file_path(volume_id).await?).await? {
            return Err(ControllerError::NotFound(volume_id.to_string()));
        }
        let metadata = self.load_metadata(volume_id).await?;
        if let Some(metadata) = metadata {
            if let Some(attached_node) = &metadata.attached_node
                && attached_node != node_id
            {
                return Err(ControllerError::AlreadyAttached {
                    volume_id: volume_id.to_string(),
                    attached_node: attached_node.clone(),
                    requested_node: node_id.to_string(),
                });
            }
            if metadata.attached_node.is_none() {
                self.save_metadata(&Metadata {
                    volume_id: volume_id.to_string(),
                    attached_node: Some(node_id.to_string()),
                })
                .await?;
            }
        } else {
            let metadata = Metadata {
                volume_id: volume_id.to_string(),
                attached_node: Some(node_id.to_string()),
            };
            self.save_metadata(&metadata).await?;
        }
        Ok(())
    }

    pub async fn unpublish_volume(
        &self,
        volume_id: &str,
        node_id: &str,
    ) -> Result<(), ControllerError> {
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

    #[tokio::test]
    async fn local_volume_lifecycle() {
        let root = std::env::temp_dir().join(format!(
            "loop-csi-test-{}-{}",
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
        let id = format!("{url}:pvc-123");
        let operator = ControllerOperator::new(1024, mounts.to_string_lossy().to_string())
            .await
            .unwrap();

        assert_eq!(
            operator.create_volume(&id, Some(2048), &url).await.unwrap(),
            2048
        );
        assert_eq!(
            operator.create_volume(&id, Some(1024), &url).await.unwrap(),
            2048
        );
        assert_eq!(operator.volume_size(&id).await.unwrap(), 2048);
        assert!(matches!(
            operator.create_volume(&id, Some(4096), &url).await,
            Err(ControllerError::ExistingSize(..))
        ));
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
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
