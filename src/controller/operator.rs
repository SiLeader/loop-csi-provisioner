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
        self.mounter.mount(url, &self.base_directory).await?;

        let size = capacity.unwrap_or(self.default_size);

        let path = self.volume_file_path(volume_id).await?;
        let file = tokio::fs::OpenOptions::new()
            .create(true)
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
        } else {
            let metadata = Metadata {
                volume_id: volume_id.to_string(),
                attached_node: Some(node_id.to_string()),
            };
            self.save_metadata(&metadata).await?;
        }
        Ok(())
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
        let size = size.unwrap_or(self.default_size);
        file.set_len(size as u64).await?;
        Ok(size)
    }
}
