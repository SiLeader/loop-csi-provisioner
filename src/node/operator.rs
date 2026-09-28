use crate::mount::MountManager;
use crate::node::error::NodeError;
use crate::volume_id::{VOLUME_DIR, parse_volume_id};
use std::path::PathBuf;
use tokio::process::Command;

pub(crate) struct NodeOperator {
    base_directory: String,
    mounter: MountManager,
}

impl NodeOperator {
    pub async fn new(base_directory: String) -> std::io::Result<Self> {
        Ok(Self {
            base_directory,
            mounter: MountManager::default(),
        })
    }

    async fn volume_file_path(&self, volume_id: &str) -> Result<PathBuf, NodeError> {
        let (url, volume_id) = parse_volume_id(volume_id).ok_or(NodeError::VolumeIdParse)?;

        let mount_point = self.mounter.mount(url, &self.base_directory).await?;
        let storage_base_directory = PathBuf::from(&mount_point).join(VOLUME_DIR);
        Ok(storage_base_directory.join(format!("{}.img", volume_id)))
    }

    async fn find_loop_device(&self, volume_id: &str) -> Result<String, NodeError> {
        let file_name = self
            .volume_file_path(volume_id)
            .await?
            .to_string_lossy()
            .to_string();
        let res = Command::new("losetup")
            .args(["--find", "--show", &file_name])
            .output()
            .await?;
        if !res.status.success() {
            return Err(res.into());
        }
        let loop_device = String::from_utf8_lossy(&res.stdout).trim().to_string();
        Ok(loop_device)
    }

    pub async fn stage_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
    ) -> Result<(), NodeError> {
        let loop_device = self.find_loop_device(volume_id).await?;

        let res = Command::new("blkid").args([&loop_device]).output().await?;
        if !res.status.success() {
            return Err(res.into());
        }
        if res.stdout.is_empty() {
            let res = Command::new("mkfs.ext4")
                .args(["-F", &loop_device])
                .output()
                .await?;
            if !res.status.success() {
                return Err(res.into());
            }
        }

        let res = Command::new("mount")
            .args([&loop_device, staging_target_path])
            .output()
            .await?;
        if res.status.success() {
            Ok(())
        } else {
            Err(res.into())
        }
    }

    pub async fn publish_volume(
        &self,
        staging_target_path: &str,
        target_path: &str,
    ) -> Result<(), NodeError> {
        let res = Command::new("mount")
            .args([staging_target_path, target_path])
            .output()
            .await?;
        if res.status.success() {
            Ok(())
        } else {
            Err(res.into())
        }
    }

    pub async fn expand_volume(&self, volume_id: &str) -> Result<i64, NodeError> {
        let path = self.volume_file_path(volume_id).await?;
        if !tokio::fs::try_exists(&path).await? {
            return Err(NodeError::CommandFailure {
                code: -1,
                message: format!("Volume {} not found", volume_id),
            });
        }

        let loop_device = self.find_loop_device(volume_id).await?;

        let res = Command::new("losetup")
            .args(["-c", &loop_device])
            .output()
            .await?;
        if !res.status.success() {
            return Err(res.into());
        }

        let res = Command::new("resize2fs")
            .args([&loop_device])
            .output()
            .await?;
        if !res.status.success() {
            return Err(res.into());
        }

        let metadata = tokio::fs::metadata(&path).await?;
        Ok(metadata.len() as i64)
    }
}
