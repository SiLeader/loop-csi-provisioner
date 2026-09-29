use crate::mount::MountManager;
use crate::node::error::NodeError;
use crate::syscall::{Filesystem, MountOptions, MountSource, Syscall};
use crate::volume_id::{VOLUME_DIR, parse_volume_id};
use std::path::PathBuf;
use tokio::process::Command;

pub(crate) struct NodeOperator {
    base_directory: String,
    mounter: MountManager,
    syscall: Syscall,
}

impl NodeOperator {
    pub async fn new(base_directory: String) -> std::io::Result<Self> {
        Ok(Self {
            base_directory,
            mounter: MountManager::default(),
            syscall: Syscall::default(),
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
        let loop_dev = self.syscall.find_loop(&file_name).await?;

        Ok(loop_dev.to_string_lossy().to_string())
    }

    pub async fn stage_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
    ) -> Result<(), NodeError> {
        if self.is_mount_point(staging_target_path).await? {
            return Ok(());
        }
        tokio::fs::create_dir_all(staging_target_path).await?;
        let loop_device = self.find_loop_device(volume_id).await?;

        let fs = match self.syscall.detect_filesystem(&loop_device).await? {
            None => {
                let res = Command::new("mkfs.ext4")
                    .args(["-F", &loop_device])
                    .output()
                    .await?;
                if !res.status.success() {
                    return Err(res.into());
                }
                Filesystem::Ext4
            }
            Some(fs) => {
                if fs != Filesystem::Ext4 {
                    return Err(NodeError::UnsupportedFilesystem);
                }
                fs
            }
        };

        self.syscall
            .mount(
                MountSource::fs(fs, &loop_device),
                staging_target_path,
                MountOptions::default(),
            )
            .await?;
        Ok(())
    }

    pub async fn publish_volume(
        &self,
        staging_target_path: &str,
        target_path: &str,
        readonly: bool,
    ) -> Result<(), NodeError> {
        if self.is_mount_point(target_path).await? {
            return Ok(());
        }
        tokio::fs::create_dir_all(target_path).await?;

        let src = MountSource::bind(staging_target_path);
        self.syscall
            .mount(src.clone(), target_path, MountOptions::default())
            .await?;
        if readonly {
            self.syscall
                .mount(
                    src,
                    target_path,
                    MountOptions::default().remount(true).readonly(true),
                )
                .await?;
        }
        Ok(())
    }

    async fn is_mount_point(&self, path: &str) -> Result<bool, NodeError> {
        let res = self.mounter.check_mount_point(path).await?;
        Ok(res)
    }

    pub async fn unpublish_volume(&self, target_path: &str) -> Result<(), NodeError> {
        if self.is_mount_point(target_path).await? {
            self.syscall.unmount(target_path).await?;
        }
        Ok(())
    }

    pub async fn unstage_volume(
        &self,
        volume_id: &str,
        staging_target_path: &str,
    ) -> Result<(), NodeError> {
        if self.is_mount_point(staging_target_path).await? {
            self.syscall.unmount(staging_target_path).await?;
        }
        let path = self.volume_file_path(volume_id).await?;
        let path = path.to_string_lossy().to_string();
        let existing = Command::new("losetup").args(["-j", &path]).output().await?;
        if !existing.status.success() {
            return Err(existing.into());
        }
        if let Some(device) = String::from_utf8_lossy(&existing.stdout)
            .lines()
            .next()
            .and_then(|line| line.split_once(':').map(|(device, _)| device.to_string()))
        {
            self.syscall.detach_loop(&device).await?;
        }
        Ok(())
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
        self.syscall
            .apply_loop_device_capacity(&loop_device)
            .await?;

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
